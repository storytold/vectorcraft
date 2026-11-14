//! The parameter keys a command accepts, read from its params description (#1442).
//!
//! A description starts with the params object: `{x, width?: pt, name?|title?: …, sides=6, …}`, or
//! with the objects a command takes instead of one another: `{path} or {name, dataBase64}`,
//! `{name} | {names: […]}`. Each entry of an object starts with a key, or with alternatives split
//! by `|`; a `?` marks an optional one, and a `:`, `=` or `(` starts what the entry says about it. An entry that starts
//! with `…` stands for keys listed elsewhere: those of each command it names after `as` or `of`
//! (`…settings as file.new`), or, naming none, any key (the options of the format an export picks). Objects inside a value aren't
//! described key by key, so only the top level is checked, except in the options of an export, a
//! save or Export for Screens: those are checked against the options of the formats the command
//! can write, in `marks`, `bleed` and the other objects too ([`check_export_options`]); and in the
//! objects of an array whose keys `MEMBER_KEYS` lists, such as the `items` of
//! `object.setLiveShape` and `addon.pathtools.cornerSet`.
//!
//! [`Session::execute`] answers a key the description doesn't name with a parameter error
//! ([`check_params`]), for every caller (the app, the control channel, MCP, the CLI) and for the
//! commands a command runs, the steps of a `command.batch` or an `action.batch` included: a
//! misspelled `widht` no longer runs the command with its default. So are the `params` of an
//! effect (#1592): `effect.apply` and
//! `effect.setParams` answer a key the effect doesn't take, or a value of another type than the
//! effect reads, with a parameter error ([`check_effect_params`]).
//!
//! A command with a parameter schema (`CommandSpec::schema`) is also checked against the schema
//! before it runs ([`crate::schemas::check`]): the schema refuses a value of the wrong type or out
//! of range, a missing required parameter and an unknown key at any depth, and fills in declared
//! defaults, so the command runs with the filled-in params. The check here still runs first for
//! such a command, so a misspelled top-level key gets the same error from either, and a
//! `command.batch` or an `action.batch` checks each step's schema before the first step runs. The
//! keys of commands without a schema, and the options of exports and effects, are checked only as
//! this module describes.
//!
//! Saved steps play through [`Session::replay`] instead: a journal or an action recorded by an older
//! version, with a key a command no longer reads or an effect parameter it no longer takes, still
//! plays. A key a schema doesn't name passes there too, and the values of the parameters it names
//! are still checked. Even there, an effect's `color` that the effect can't read and a Levels or Curves
//! `channel` that the effect doesn't have are refused by `effect.apply` and `effect.setParams`
//! themselves ([`check_effect_color`], [`check_effect_channel`]).

use serde_json::Value;
use vectorcraft_apischema::shown_key;

use super::{bad, command_specs, find_command};
use crate::Result;

/// What a params description says about the keys of the params object.
#[derive(Debug, Default, PartialEq)]
pub struct ParamKeys<'a> {
    /// The keys it names.
    pub keys: Vec<&'a str>,
    /// Its `…` entries, without the `…`.
    pub more: Vec<&'a str>,
}

/// The text between the braces of the object `doc` starts with and what follows it, or why it
/// has none.
pub(crate) fn object_body(doc: &str) -> std::result::Result<(&str, &str), String> {
    // (Named, not written in the `Err`: these are for the tests of the descriptions, not messages
    // the app shows, which the message catalogs would have to translate.)
    let (no_object, not_closed) = ("the description doesn't start with the params object `{…}`", "the params object `{…}` isn't closed");
    if !doc.starts_with('{') {
        return Err(no_object.into());
    }
    let (mut depth, mut quote) = (0usize, None);
    for (i, c) in doc.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '`') => quote = Some(c),
            (None, '{' | '[' | '(') => depth += 1,
            (None, '}' | ']' | ')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok((doc.get(1..i).unwrap_or_default(), doc.get(i + 1..).unwrap_or_default()));
                }
            }
            _ => {}
        }
    }
    Err(not_closed.into())
}

/// `s` split at each `sep` that is outside brackets, quotes and backticks.
pub(crate) fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut quote, mut start) = (Vec::new(), 0usize, None, 0);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '`') => quote = Some(c),
            (None, '{' | '[' | '(') => depth += 1,
            (None, '}' | ']' | ')') => depth = depth.saturating_sub(1),
            (None, _) if c == sep && depth == 0 => {
                out.push(s.get(start..i).unwrap_or_default());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(s.get(start..).unwrap_or_default());
    out
}

pub(crate) fn is_key(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The keys a params description names, or what in it doesn't read as one.
pub fn doc_keys(doc: &str) -> std::result::Result<ParamKeys<'_>, String> {
    let mut out = ParamKeys::default();
    let (mut body, mut rest) = object_body(doc)?;
    let mut entries = split_top(body, ',');
    // `{…} or {…}`, `{…} | {…}`: the objects the command takes instead.
    while let Some(next) = rest.trim_start().strip_prefix("or ").or_else(|| rest.trim_start().strip_prefix("| ")).filter(|r| r.starts_with('{')) {
        (body, rest) = object_body(next)?;
        entries.extend(split_top(body, ','));
    }
    for entry in entries.into_iter().map(str::trim).filter(|e| !e.is_empty()) {
        if let Some(more) = entry.strip_prefix('…') {
            out.more.push(more.trim());
            continue;
        }
        // `a?|b?: note` names two keys, and so does `a?: note | b?: note`; in `a?: x|y` the `y`
        // is a value.
        let mut noted = false;
        for alternative in split_top(entry, '|') {
            let head = alternative.split([':', '=', '(']).next().unwrap_or_default();
            let rest = alternative.get(head.len()..).unwrap_or_default();
            let name = head.trim().trim_end_matches('?').trim_end();
            if !noted {
                if !is_key(name) {
                    let start: String = entry.chars().take(60).collect();
                    let why = format!("`{start}` doesn't start with a parameter name");
                    return Err(why);
                }
                out.keys.push(name);
                noted = !rest.is_empty();
            } else if is_key(name) && (head.trim_end().ends_with('?') || rest.starts_with(':')) {
                out.keys.push(name);
            }
        }
    }
    Ok(out)
}

/// The commands whose keys a `…` entry stands for: those it names after `as` or `of` (`settings as
/// file.new`, `the PDF options of document.exportPdf`), not one it only points to (`see …`).
fn named_commands(more: &str) -> impl Iterator<Item = &'static super::CommandSpec> + '_ {
    let words: Vec<&str> = more.split_whitespace().collect();
    let id = |w: &str| find_command(w.trim_matches(|c: char| !c.is_ascii_alphanumeric()));
    (1..words.len()).filter_map(move |i| id(words.get(i)?).filter(|_| matches!(words.get(i - 1), Some(&"as" | &"of"))))
}

/// Every key command `doc` accepts, the keys of the commands its `…` entries name included.
/// `None`: any key (a `…` entry naming no command, or a description that doesn't parse, which
/// the tests rule out).
fn accepted(doc: &'static str, depth: usize) -> Option<Vec<&'static str>> {
    let parsed = doc_keys(doc).ok()?;
    let mut keys = parsed.keys;
    for more in parsed.more {
        let mut named = named_commands(more).peekable();
        // A reference that leads back here would never end: descriptions refer one or two deep.
        if named.peek().is_none() || depth > 4 {
            return None;
        }
        for spec in named {
            // A command that isn't checked yet doesn't vouch for the ones that take its keys.
            if UNCHECKED.contains(&spec.id) {
                return None;
            }
            keys.extend(accepted(spec.params, depth + 1)?);
        }
    }
    Some(keys)
}

/// Every key the description `doc` takes, those of the commands its `…` entries name included, or
/// `None` when that isn't known: it doesn't parse, a `…` entry names no command, or one names a
/// command in [`UNCHECKED`].
pub(crate) fn accepted_keys(doc: &'static str) -> Option<Vec<&'static str>> {
    accepted(doc, 0)
}

/// Does the description `doc` name `key` as a parameter (its own keys, and those of the commands
/// its `…` entries name)?
pub fn takes_key(doc: &'static str, key: &str) -> bool {
    doc_keys(doc).is_ok_and(|parsed| parsed.keys.contains(&key)) || accepted(doc, 0).is_some_and(|keys| keys.contains(&key))
}

/// Does the description `doc` take `id` as `ids` of one object (`{ids?|id?}`, `{id?, ids?: …
/// (instead of id)}`, `{id} or {ids: […]}`): it names both keys, and no params object of it names
/// `ids` beside a required `id`, which is something else (`plugin.run {id, ids?}`: the plug-in's)?
pub(crate) fn id_stands_for_ids(doc: &'static str) -> bool {
    fn head(alternative: &str) -> &str {
        alternative.split([':', '=', '(']).next().unwrap_or_default().trim()
    }
    if !takes_key(doc, "ids") || !takes_key(doc, "id") {
        return false;
    }
    let mut rest = doc;
    while let Ok((body, after)) = object_body(rest) {
        let entries = split_top(body, ',');
        let required_id = entries.iter().any(|e| split_top(e, '|').len() == 1 && head(e) == "id");
        let names_ids = entries.iter().any(|e| split_top(e, '|').iter().any(|a| head(a).trim_end_matches('?').trim_end() == "ids"));
        if required_id && names_ids {
            return false;
        }
        match after.trim_start().strip_prefix("or ").or_else(|| after.trim_start().strip_prefix("| ")).filter(|r| r.starts_with('{')) {
            Some(next) => rest = next,
            None => break,
        }
    }
    true
}

/// The edits (insert, delete, replace, swap of neighbours) that turn `a` into `b`, ignoring case.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().map(|c| c.to_ascii_lowercase()).collect(), b.chars().map(|c| c.to_ascii_lowercase()).collect());
    let (mut before, mut prev): (Vec<usize>, Vec<usize>) = (Vec::new(), (0..=b.len()).collect());
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let at = |r: &[usize], k: usize| r.get(k).copied().unwrap_or(usize::MAX / 2);
            let mut d = (at(&prev, j) + usize::from(ca != cb)).min(at(&prev, j + 1) + 1).min(at(&row, j) + 1);
            if i > 0 && j > 0 && a.get(i - 1) == Some(cb) && b.get(j - 1) == Some(ca) {
                d = d.min(at(&before, j - 1) + 1);
            }
            row.push(d);
        }
        before = std::mem::replace(&mut prev, row);
    }
    prev.last().copied().unwrap_or(0)
}

/// The key of `keys` a misspelt `key` most likely meant: the nearest one, when it is near.
fn nearest<'a>(key: &str, keys: &[&'a str]) -> Option<&'a str> {
    let len = key.chars().count();
    let most = (len / 3).max(1);
    // A key whose length differs by more than `most` is more than `most` edits away and isn't
    // measured.
    let near = keys.iter().filter(|k| k.chars().count().abs_diff(len) <= most).map(|k| (distance(key, k), *k)).min()?;
    (near.0 <= most).then_some(near.1)
}

/// `params` has only keys the description `doc` of command `id` names, or the parameter error
/// naming the first one it doesn't, with the nearest accepted key or the accepted ones.
pub fn check_keys(id: &str, doc: &'static str, params: &Value) -> Result<()> {
    let Value::Object(given) = params else { return Ok(()) };
    if given.is_empty() {
        return Ok(());
    }
    let Some(keys) = accepted(doc, 0) else { return Ok(()) };
    match given.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(key) => Err(unknown(id, key, &keys)),
        None => Ok(()),
    }
}

/// Parameters a command takes instead of one another: (command, key, other key). Giving both,
/// neither null, is a parameter error ([`check_exclusive_keys`]).
const EXCLUSIVE_KEYS: &[(&str, &str, &str)] = &[("addon.pathtools.cornerSet", "items", "ids"), ("addon.pathtools.cornerRemove", "items", "ids")];

/// `params` of command `id` give at most one of each pair in [`EXCLUSIVE_KEYS`], or the parameter
/// error naming the pair.
fn check_exclusive_keys(id: &str, params: &Value) -> Result<()> {
    let given = |key: &str| params.get(key).filter(|v| !v.is_null());
    for (_, key, other) in EXCLUSIVE_KEYS.iter().filter(|(c, ..)| *c == id) {
        if given(key).is_some() && given(other).is_some() {
            return Err(bad(id, format!("give `{key}` or `{other}`, not both")));
        }
    }
    Ok(())
}

/// The parameter error of command `id` for `key`, which isn't one of `keys`: with the nearest of
/// them, or all of them.
pub(crate) fn unknown(id: &str, key: &str, keys: &[&str]) -> crate::EngineError {
    let near = nearest(key, keys);
    let key = shown_key(key);
    bad(
        id,
        match near {
            Some(near) => format!("unknown parameter `{key}`; did you mean `{near}`?"),
            None if keys.is_empty() => format!("unknown parameter `{key}`; the command takes no parameters"),
            None => format!("unknown parameter `{key}`; the command takes: {}", keys.join(", ")),
        },
    )
}

/// Commands whose params hold an array of objects with keys of their own: the command, the key of
/// the array and the keys each of its objects takes.
const MEMBER_KEYS: &[(&str, &str, &[&str])] = &[
    ("object.setLiveShape", "items", &["id", "corners"]),
    ("addon.pathtools.cornerSet", "items", &["id", "anchors"]),
    ("addon.pathtools.cornerRemove", "items", &["id", "anchors"]),
];

/// Each object in the arrays [`MEMBER_KEYS`] names for command `id` has only the keys it lists, or
/// the parameter error naming the first other one with its place (`items[0].corner`), and the
/// nearest accepted key or the accepted ones. A member that isn't an object is the command's to
/// report.
fn check_member_keys(id: &str, params: &Value) -> Result<()> {
    for (_, list, keys) in MEMBER_KEYS.iter().filter(|(c, ..)| *c == id) {
        let Some(members) = params.get(*list).and_then(Value::as_array) else { continue };
        for (i, member) in members.iter().enumerate() {
            let Value::Object(given) = member else { continue };
            let Some(key) = given.keys().find(|k| !keys.contains(&k.as_str())) else { continue };
            let at = |k: &str| format!("{list}[{i}].{k}");
            let near = nearest(key, keys);
            let key = at(&shown_key(key));
            return Err(bad(
                id,
                match near {
                    Some(near) => {
                        let near = at(near);
                        format!("unknown parameter `{key}`; did you mean `{near}`?")
                    }
                    None => {
                        let keys = keys.iter().map(|k| at(k)).collect::<Vec<_>>().join(", ");
                        format!("unknown parameter `{key}`; the command takes: {keys}")
                    }
                },
            ));
        }
    }
    Ok(())
}

/// `params` of built-in effect `effect`, as command `id` was given them (#1592): only keys the
/// effect takes (those of its dialog defaults and of its params description), each of the type
/// the effect reads ([`check_effect_types`]), a `color` the effect can draw ([`check_effect_color`])
/// and a Levels or Curves `channel` the effect has ([`check_effect_channel`]). Numbers out of range
/// pass: the effect clamps them when it draws. An installed effect plug-in's are checked against its
/// manifest, and an unknown effect is the command's to report.
pub fn check_effect_params(id: &str, effect: &str, params: &Value) -> Result<()> {
    let Value::Object(given) = params else { return Ok(()) };
    if effect.starts_with(vectorcraft_plugins::effect::PREFIX) {
        return check_effect_types(id, effect, params);
    }
    let Some(info) = vectorcraft_render::effects::effect_info(effect) else { return Ok(()) };
    let defaults = info.defaults.as_object();
    let mut keys: Vec<&str> = defaults.into_iter().flat_map(|d| d.keys().map(String::as_str)).collect();
    // A description that doesn't parse names no more keys (the tests rule that out).
    for key in doc_keys(info.params).map(|d| d.keys).unwrap_or_default() {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if let Some(key) = given.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(unknown(id, key, &keys));
    }
    check_effect_types(id, effect, params)?;
    // An SVG filter's XML is one drawable <filter> element.
    if effect == vectorcraft_render::effects::SVG_FILTER
        && let Some(xml) = given.get("xml")
    {
        let xml = xml.as_str().ok_or_else(|| bad(id, "`xml` must be a string"))?;
        vectorcraft_render::effects::svg_filter::check(xml).map_err(|e| bad(id, e))?;
    }
    if effect == vectorcraft_render::effects::SVG_FILTER
        && let Some(scale) = given.get("scale")
        && !scale.as_f64().is_some_and(|k| (0.001..=1000.0).contains(&k))
    {
        return Err(bad(id, "`scale` must be a number from 0.001 to 1000"));
    }
    if effect == vectorcraft_render::effects::SVG_FILTER
        && let Some(origin) = given.get("origin")
        && !origin.as_array().is_some_and(|o| o.len() == 2 && o.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)))
    {
        return Err(bad(id, "`origin` must be [x, y] in points"));
    }
    // Its name is the filter's id.
    if effect == vectorcraft_render::effects::SVG_FILTER && given.contains_key("name") && !given.contains_key("xml") {
        return Err(bad(id, "an SVG filter is named by its id: give `xml` with `name`"));
    }
    // A Warp's map is one this build draws: 0, the earlier map, or its style's Bézier net.
    let newest = vectorcraft_render::effects::WARP_NET;
    if effect.starts_with("warp.")
        && let Some(v) = given.get("version")
        && vectorcraft_render::effects::warp_version(v).is_none()
    {
        return Err(bad(id, format!("`version` must be 0 or {newest}")));
    }
    check_effect_color(id, effect, params)?;
    check_effect_channel(id, effect, params)
}

/// The type of the values in `params` of effect `effect`, as command `id` was given them: a key
/// the effect has a dialog default for takes a value of its default's type, a key only the
/// effect's params description names takes the type the effect reads ([`undefaulted_type`]), and
/// an installed effect plug-in's take the types its manifest gives them. The `style` of Crop Marks
/// is `roman` or `japanese`. A number given as text (`"10 pt"`), `true` for a number, `1` for a
/// switch and `null` for either are refused.
fn check_effect_types(id: &str, effect: &str, params: &Value) -> Result<()> {
    let Value::Object(given) = params else { return Ok(()) };
    if effect.starts_with(vectorcraft_plugins::effect::PREFIX) {
        return match vectorcraft_plugins::effect::installed(effect) {
            Some(plugin) => plugin.manifest().resolve_params(params).map(drop).map_err(|why| bad(id, why.to_string())),
            None => Ok(()),
        };
    }
    let Some(info) = vectorcraft_render::effects::effect_info(effect) else { return Ok(()) };
    for (key, v) in given {
        let want = info.defaults.get(key).cloned().or_else(|| undefaulted_type(effect, key));
        match want {
            Some(Value::Number(_)) if !v.is_number() => return Err(bad(id, format!("`{key}` must be a number"))),
            Some(Value::Bool(_)) if !v.is_boolean() => return Err(bad(id, format!("`{key}` must be true or false"))),
            // (A colour is also taken as `[r, g, b]` or as a colour object.)
            Some(Value::String(_)) if v.is_number() || v.is_boolean() => return Err(bad(id, format!("`{key}` must be a string"))),
            _ => {}
        }
    }
    // Crop Marks draw one of two styles.
    if effect == vectorcraft_render::effects::CROP_MARKS
        && let Some(style) = given.get("style")
        && style.as_str().and_then(vectorcraft_doc::marks::MarkStyle::parse).is_none()
    {
        return Err(bad(id, "`style` must be roman or japanese"));
    }
    Ok(())
}

/// A value of the type effect `effect` reads for `key`, a parameter its params description names
/// without a dialog default: the earlier 3D Perspective and light (`perspective`, `lightAzimuth`,
/// `lightElevation`) and Scribble `variation` are numbers, an Outline Stroke's `width` is a number
/// and a Gaussian Blur's `linear` is true or false. A Glass or Texturizer `textureImage`, an SVG
/// filter's `name` and `xml`, a Warp's `version` and the `style` of Crop Marks have checks of their
/// own; they aren't listed.
pub(crate) fn undefaulted_type(effect: &str, key: &str) -> Option<Value> {
    use vectorcraft_render::effects;
    let number = || Some(Value::from(0.0));
    match key {
        "perspective" | "lightAzimuth" | "lightElevation" if effects::is_three_d(effect) => number(),
        "variation" if effect == "stylize.scribble" => number(),
        "width" if effect == "path.outlineStroke" => number(),
        "linear" if effect == "blur.gaussian" => Some(Value::Bool(false)),
        _ => None,
    }
}

/// Can an effect read `v` as its `color`: a string [`Color::from_hex`] reads (`#rrggbb`, `#rgb`,
/// `#rrggbbaa`, black, white, red, green, blue), `[r, g, b]` whose first three entries are numbers
/// (read on a 0..1 scale) or a color object (`{"model": "rgb", "r": …}`)? The effect draws its
/// default color for any other value, except that an array of three or more entries draws an
/// entry that isn't a number as 0.
///
/// [`Color::from_hex`]: vectorcraft_color::Color::from_hex
fn is_effect_color(v: &Value) -> bool {
    match v {
        Value::String(s) => vectorcraft_color::Color::from_hex(s).is_some(),
        Value::Array(a) => a.len() >= 3 && a.iter().take(3).all(Value::is_number),
        Value::Object(_) => serde_json::from_value::<vectorcraft_color::Color>(v.clone()).is_ok(),
        _ => false,
    }
}

/// The `color` in `params` of built-in effect `effect`, when the effect has a `color` parameter,
/// as command `id` was given it: a parameter error naming it unless the effect can read it
/// ([`is_effect_color`]). `effect.apply` and `effect.setParams` run this themselves before they
/// edit, so the app's own edits, the steps of a `command.batch` and a replayed journal are held to
/// it too.
pub(crate) fn check_effect_color(id: &str, effect: &str, params: &Value) -> Result<()> {
    let key = "color";
    let Some(v) = params.get(key) else { return Ok(()) };
    if effect_takes_color(effect) && !is_effect_color(v) {
        return Err(bad(id, format!("`{key}` must be a #rrggbb color")));
    }
    Ok(())
}

/// Does built-in effect `effect` have a `color` parameter (one whose default is a string)? Plug-in
/// effects have none here.
pub(crate) fn effect_takes_color(effect: &str) -> bool {
    !effect.starts_with(vectorcraft_plugins::effect::PREFIX)
        && vectorcraft_render::effects::effect_info(effect).is_some_and(|info| info.defaults.get("color").is_some_and(Value::is_string))
}

/// The color that built-in effect `effect` draws with `params`: their `color`, else the effect's
/// default color. `None` when the effect has no `color` parameter ([`effect_takes_color`]) or
/// `params` hold a `color` it can't read ([`is_effect_color`]).
pub(crate) fn effect_color(effect: &str, params: &Value) -> Option<vectorcraft_color::Color> {
    if !effect_takes_color(effect) || params.get("color").is_some_and(|v| !is_effect_color(v)) {
        return None;
    }
    let merged = vectorcraft_render::effects::merged_params(effect, params);
    Some(vectorcraft_render::effects::color_param(&merged, vectorcraft_color::Color::BLACK))
}

/// The `channel` in `params` of Levels or Curves (`effect`), as command `id` was given it: a
/// parameter error naming the channels unless the effect reads it
/// ([`vectorcraft_render::effects::is_channel`]); the effect draws any other value as RGB.
/// `effect.apply` and `effect.setParams` run this themselves before they edit, as they run
/// [`check_effect_color`].
pub(crate) fn check_effect_channel(id: &str, effect: &str, params: &Value) -> Result<()> {
    let key = "channel";
    let Some(v) = params.get(key) else { return Ok(()) };
    if !matches!(effect, "adjust.levels" | "adjust.curves") || v.as_str().is_some_and(vectorcraft_render::effects::is_channel) {
        return Ok(());
    }
    Err(bad(id, format!("`{key}` must be one of {}", vectorcraft_render::effects::CHANNELS.join(", "))))
}

/// Commands whose params aren't checked yet (#1442): they read keys in ways the audit of the
/// descriptions couldn't follow (a list of keys in a loop, a whole object read as settings), so
/// their descriptions aren't proven to name every key they take. Sorted; take a command out once
/// its description is checked against its code.
pub const UNCHECKED: &[&str] = &[
    "appearance.copyFrom",
    "appearance.setItem",
    "artboard.rearrange",
    "assets.export",
    "assets.settings.set",
    "attributes.info",
    "attributes.set",
    "brush.freehand",
    "css.export",
    "css.generate",
    "css.selection",
    "document.dxfInfo",
    "document.exportDxf",
    "document.exportEps",
    "document.exportForScreens",
    "document.exportForWeb",
    "document.exportForWeb.preview",
    "document.exportPdf",
    "document.find",
    "document.inspect",
    "document.node",
    "document.open",
    "document.pdfInfo",
    "document.pdfSettings",
    "document.rasterEffectsSettings",
    "edit.assignProfile",
    "edit.colors.adjustBalance",
    "edit.colors.invert",
    "edit.colors.saturate",
    "edit.colors.toCMYK",
    "edit.colors.toGrayscale",
    "edit.colors.toRGB",
    "edit.paste",
    "edit.pasteInBack",
    "edit.pasteInFront",
    "edit.pasteInPlace",
    "edit.pasteOnAllArtboards",
    "edit.pasteWithoutFormatting",
    "effect.apply",
    "eyedropper.setOptions",
    "file.info",
    "file.newFromTemplate",
    "file.newPresets.save",
    "file.place",
    "file.place.info",
    "file.place.queue",
    "flattener.presets.save",
    "flattener.preview",
    "graph.design",
    "graph.setType",
    "graphicStyle.loadLibrary",
    "layer.new",
    "layer.newSublayer",
    "layer.releaseToLayersBuild",
    "layer.setProps",
    "library.add",
    "library.deleteGroup",
    "library.moveItem",
    "library.removeItem",
    "library.renameGroup",
    "library.use",
    "links.placementOptions",
    "livePaint.fill",
    "livePaint.strokeEdge",
    "magicWand.set",
    "object.align",
    "object.blend.make",
    "object.blend.options",
    "object.distribute",
    "object.distributeSpacing",
    "object.expand",
    "object.flattenTransparency",
    "object.liquify",
    "object.path.splitIntoGrid",
    "object.puppetWarp",
    "object.rasterize",
    "object.repeat.options",
    "object.setBounds",
    "object.setOverprint",
    "object.setProps",
    "object.slice.divide",
    "object.slice.options",
    "object.transformAgain",
    "paint.complement",
    "paint.editGradient",
    "paint.freeform.addLine",
    "paint.freeform.addPoint",
    "paint.freeform.deletePoint",
    "paint.freeform.setPoint",
    "paint.freeform.splitLine",
    "paint.invert",
    "paint.lastColor",
    "paint.lastGradient",
    "paint.setFill",
    "paint.setGradientGeom",
    "path.appendAnchor",
    "pattern.options",
    "pdf.preset.save",
    "perspective.grid.define",
    "perspective.presets.save",
    "print.tiling",
    "recolor.apply",
    "select.editSaved",
    "shape.flare",
    "shape.polarGrid",
    "shape.rectangularGrid",
    "stroke.setAdvanced",
    "swatch.edit",
    "swatch.library.load",
    "text.setFormat",
    "text.setRangeStyle",
    "text.setStyle",
    "text.tabs.clear",
    "text.tabs.get",
    "text.tabs.set",
    "transparency.disableOpacityMask",
    "transparency.enableOpacityMask",
    "transparency.linkOpacityMask",
    "transparency.set",
    "transparency.setOpacityMask",
    "transparency.unlinkOpacityMask",
    "type.pathOptions",
    "webExport.presets.save",
    "webExport.settings",
];

/// The object options of a PDF export, key by key: the Save PDF settings as a preset stores them,
/// with the passwords it leaves out.
fn pdf_option_tree() -> Value {
    let mut tree = serde_json::to_value(vectorcraft_pdf::PdfSettings::default()).unwrap_or_default();
    if let Some(security) = tree.get_mut("security").and_then(Value::as_object_mut) {
        security.extend(["openPassword", "permissionsPassword"].map(|k| (k.to_string(), Value::Null)));
    }
    tree
}

/// `given`, the object at `within` (`marks.`), has only keys of `schema`, the objects inside it too.
fn check_option_tree(id: &str, within: &str, given: &serde_json::Map<String, Value>, schema: &serde_json::Map<String, Value>) -> Result<()> {
    for (key, value) in given {
        match (schema.get(key), value) {
            (None, _) => {
                let keys: Vec<&str> = schema.keys().map(String::as_str).collect();
                let why = match nearest(key, &keys) {
                    Some(near) => format!("did you mean `{within}{near}`?"),
                    None => format!("`{}` takes: {}", within.trim_end_matches('.'), keys.join(", ")),
                };
                let key = shown_key(key);
                return Err(bad(id, format!("unknown parameter `{within}{key}`; {why}")));
            }
            (Some(Value::Object(inner)), Value::Object(value)) => check_option_tree(id, &format!("{within}{key}."), value, inner)?,
            _ => {}
        }
    }
    Ok(())
}

/// `given`, the options object at `within` (`""` for the params themselves, `options.`,
/// `formats[0].`), has only `keys`, and only their own keys in the PDF and EPS object options
/// (`trees`). `why_not` says why a key that is not near any of `keys` is refused, and where the
/// keys are listed.
fn check_option_keys(id: &str, within: &str, given: &serde_json::Map<String, Value>, keys: &[&str], why_not: &str, trees: &Value) -> Result<()> {
    for (key, value) in given {
        if !keys.contains(&key.as_str()) {
            let near = nearest(key, keys);
            let key = shown_key(key);
            return Err(bad(
                id,
                match near {
                    Some(near) => format!("unknown parameter `{within}{key}`; did you mean `{within}{near}`?"),
                    None => format!("unknown parameter `{within}{key}`; {why_not}"),
                },
            ));
        }
        if let (Some(Value::Object(schema)), Value::Object(value)) = (trees.get(key), value) {
            check_option_tree(id, &format!("{within}{key}."), value, schema)?;
        }
    }
    Ok(())
}

/// Why an export option that no format reads is refused.
const NO_FORMAT: &str = "no format takes it (see document.formats)";

/// Why a save option that no save format reads is refused.
const NO_SAVE_FORMAT: &str = "no save format takes it (see file.formatOptions)";

/// The format ids Export for Screens writes: the writable formats, those exporter plug-ins add
/// included, but those it doesn't write.
pub fn screen_formats() -> Vec<&'static str> {
    use super::fileio::{FORMATS, SCREEN_UNWRITTEN_FORMATS, plugin_formats};
    let added = plugin_formats();
    FORMATS.iter().chain(added.iter().copied()).filter(|f| f.write && !SCREEN_UNWRITTEN_FORMATS.contains(&f.id)).map(|f| f.id).collect()
}

/// The keys a format row of Export for Screens, and the settings of a format, take: the row's own
/// (`format`, `scale`…), `includeArtboardFill` and `richBlack`, which the encoders read from a
/// row's options (`with_artboard_fills`, the render options), `params`, an exporter plug-in's own,
/// and a key some format reads, as `document.export` takes them.
fn screen_option_keys() -> Vec<&'static str> {
    use super::fileio::{FORMATS, SCREEN_ROW_KEYS};
    let mut keys: Vec<&str> = SCREEN_ROW_KEYS.to_vec();
    keys.extend(["includeArtboardFill", "richBlack", "params"]);
    keys.extend(FORMATS.iter().filter(|f| f.write).flat_map(|f| f.options).map(|o| o.name));
    keys
}

/// A format row of Export for Screens, or the settings of a format, without the keys the export
/// refuses in it ([`check_screen_rows`]), inside `marks`, `svg` and the other object options too:
/// a saved row or preset that holds a key that is no longer read loads without it.
pub fn drop_unread_row_options(row: &mut serde_json::Map<String, Value>) {
    fn retain_tree(o: &mut serde_json::Map<String, Value>, schema: &serde_json::Map<String, Value>) {
        o.retain(|k, _| schema.contains_key(k));
        for (k, v) in o.iter_mut() {
            if let (Some(Value::Object(inner)), Value::Object(v)) = (schema.get(k), v) {
                retain_tree(v, inner);
            }
        }
    }
    let keys = screen_option_keys();
    row.retain(|k, _| keys.contains(&k.as_str()));
    let trees = screen_option_trees();
    for (k, v) in row.iter_mut() {
        if let (Some(Value::Object(schema)), Value::Object(v)) = (trees.get(k), v) {
            retain_tree(v, schema);
        }
    }
}

/// [`option_trees`] of Export for Screens: its remembered settings mark PDF security whose
/// passwords were left out with `security.passwordRequired`, which the export reads.
fn screen_option_trees() -> Value {
    let mut trees = option_trees();
    if let Some(security) = trees.get_mut("security").and_then(Value::as_object_mut) {
        security.insert("passwordRequired".into(), Value::Null);
    }
    trees
}

/// The object options an export or save checks key by key: PDF's `marks`, `bleed` and the others,
/// the flattener's, and SVG's `svg` with the artboard keys it may carry.
fn option_trees() -> Value {
    let mut trees = pdf_option_tree();
    if let Some(tree) = trees.as_object_mut() {
        tree.insert("flattener".into(), serde_json::to_value(super::flatten::FlattenOptions::default()).unwrap_or_default());
        let mut svg = serde_json::to_value(vectorcraft_svg::ExportOptions::default()).unwrap_or_default();
        if let Some(svg) = svg.as_object_mut() {
            svg.extend(super::fileio::ARTBOARD_PARAMS.iter().chain(&["useArtboards"]).map(|k| (k.to_string(), Value::Null)));
        }
        tree.insert("svg".into(), svg);
    }
    trees
}

/// The format rows (`formats`) and the per-format `settings` of Export for Screens params `p`, at
/// `within` (`settings.` in `document.exportPresets.save`). A row, and the settings of a format,
/// take [`screen_option_keys`]; `settings` takes the formats Export for Screens writes.
fn check_screen_rows(id: &str, within: &str, p: &serde_json::Map<String, Value>) -> Result<()> {
    let keys = screen_option_keys();
    let trees = &screen_option_trees();
    if let Some(Value::Array(rows)) = p.get("formats") {
        for (i, row) in rows.iter().enumerate() {
            if let Value::Object(row) = row {
                check_option_keys(id, &format!("{within}formats[{i}]."), row, &keys, NO_FORMAT, trees)?;
            }
        }
    }
    if let Some(Value::Object(settings)) = p.get("settings") {
        let formats = screen_formats();
        for (format, options) in settings {
            if !formats.contains(&format.as_str()) {
                let why = match nearest(format, &formats) {
                    Some(near) => format!("did you mean `{within}settings.{near}`?"),
                    None => format!("`{within}settings` takes: {}", formats.join(", ")),
                };
                let format = shown_key(format);
                return Err(bad(id, format!("unknown parameter `{within}settings.{format}`; {why}")));
            }
            if let Value::Object(options) = options {
                check_option_keys(id, &format!("{within}settings.{format}."), options, &keys, NO_FORMAT, trees)?;
            }
        }
    }
    Ok(())
}

/// The options of an export or save command `id`. `document.export`, `document.exportSelection`
/// and `document.serialize` take a key that some format reads (one object can carry several
/// formats' options; `document.formats` lists them). `document.exportPdf`, `document.pdfSettings`
/// and `pdf.preset.save` take a key that PDF reads, `document.exportEps` one that EPS reads and
/// `document.exportDxf` one that DXF reads. `document.save`, `file.saveAs`, `file.saveCopy` and
/// `file.saveAsTemplate` take, beside the path and in `options`, a key that some save format
/// reads. `document.exportForScreens` takes the keys it reads, and checks its format rows and
/// settings ([`check_screen_rows`]), as `document.exportPresets.save` does in its `settings` and
/// `assets.export` and `assets.settings.set` do in theirs.
/// Where a command takes `marks`, `bleed`, `flattener`, `svg` or another object option of PDF,
/// EPS or SVG, only the object's own keys pass ([`option_trees`]). Saved presets are not checked:
/// one with a key that is no longer read keeps loading.
fn check_export_options(id: &str, params: &Value) -> Result<()> {
    use super::fileio::{FORMATS, SAVE_FORMATS, SCREEN_PARAM_KEYS, TEMPLATE_FORMATS, dxf, eps, pdf};
    let Value::Object(given) = params else { return Ok(()) };
    match (id, given.get("settings")) {
        ("document.exportForScreens", _) => {
            if let Some(key) = given.keys().find(|k| !SCREEN_PARAM_KEYS.contains(&k.as_str())) {
                return Err(unknown(id, key, &SCREEN_PARAM_KEYS));
            }
            return check_screen_rows(id, "", given);
        }
        ("document.exportPresets.save", Some(Value::Object(settings))) => {
            let why = format!("`settings` takes: {}", SCREEN_PARAM_KEYS.join(", "));
            check_option_keys(id, "settings.", settings, &SCREEN_PARAM_KEYS, &why, &Value::Null)?;
            return check_screen_rows(id, "settings.", settings);
        }
        // Export Assets exports with, and stores, Export for Screens' format rows and settings.
        ("assets.export" | "assets.settings.set", _) => return check_screen_rows(id, "", given),
        _ => {}
    }
    let mut keys: Vec<&str> = vec!["path", "format", "acknowledgeLoss"];
    // A save's `options` and the keys they take.
    let mut inner: Option<(&serde_json::Map<String, Value>, Vec<&str>)> = None;
    let why_not = match id {
        "document.save" | "file.saveAs" | "file.saveCopy" | "file.saveAsTemplate" => {
            let saved = FORMATS.iter().filter(|f| SAVE_FORMATS.contains(&f.id) || TEMPLATE_FORMATS.contains(&f.id));
            let options: Vec<&str> = saved.flat_map(|f| f.options).map(|o| o.name).collect();
            if let Some(Value::Object(o)) = given.get("options") {
                inner = Some((o, options.clone()));
            }
            keys.extend(["options", "modified"]);
            keys.extend(options);
            NO_SAVE_FORMAT
        }
        "document.export" | "document.exportSelection" | "document.serialize" => {
            keys.extend(super::fileio::EXPORT_PARAMS);
            keys.extend(FORMATS.iter().filter(|f| f.write).flat_map(|f| f.options).map(|o| o.name));
            NO_FORMAT
        }
        "document.exportPdf" | "document.pdfSettings" | "pdf.preset.save" => {
            keys.push("includeArtboardFill");
            keys.extend(pdf::OPTIONS.iter().map(|o| o.name));
            match id {
                "document.pdfSettings" => keys.push("includeDocument"),
                "pdf.preset.save" => keys.extend(["name", "newName", "description"]),
                _ => {}
            }
            "PDF doesn't take it (see document.formats)"
        }
        "document.exportEps" => {
            keys.push("includeArtboardFill");
            keys.extend(eps::OPTIONS.iter().map(|o| o.name));
            "EPS doesn't take it (see document.formats)"
        }
        "document.exportDxf" => {
            keys.extend(dxf::OPTIONS.iter().map(|o| o.name));
            "DXF doesn't take it (see document.formats)"
        }
        _ => return Ok(()),
    };
    let trees = option_trees();
    if let Some((o, options)) = inner {
        check_option_keys(id, "options.", o, &options, NO_SAVE_FORMAT, &trees)?;
    }
    check_option_keys(id, "", given, &keys, why_not, &trees)
}

/// [`check_keys`], [`check_exclusive_keys`] and [`check_member_keys`] for an engine command, the steps of a `command.batch` or an
/// `action.batch` included, [`check_effect_params`] for the effect `effect.apply` adds and
/// [`check_export_options`] for an export. A command the engine doesn't have passes: running it reports that.
pub fn check_params(id: &str, params: &Value) -> Result<()> {
    check_params_in(id, params, false)
}

/// [`check_params`] for a step of a batch when `in_batch`: batches can't nest, so the steps of a
/// `command.batch` or `action.batch` step aren't checked (running it reports that), however deep they go.
fn check_params_in(id: &str, params: &Value, in_batch: bool) -> Result<()> {
    let Some(spec) = find_command(id) else { return Ok(()) };
    if let ("effect.apply" | "appearance.addEffect", Some(effect), Some(given)) =
        (id, params.get("effect").or_else(|| params.get("id")).and_then(Value::as_str), params.get("params"))
    {
        check_effect_params(id, effect, given)?;
    }
    check_export_options(id, params)?;
    if UNCHECKED.contains(&id) {
        return Ok(());
    }
    check_keys(id, spec.params, params)?;
    check_exclusive_keys(id, params)?;
    check_member_keys(id, params)?;
    if in_batch {
        crate::schemas::check(spec, params, false)?;
    }
    if matches!(id, "command.batch" | "action.batch") && !in_batch {
        for step in params.get("commands").and_then(Value::as_array).into_iter().flatten() {
            if let Some(step_id) = step.get("command").and_then(Value::as_str) {
                check_params_in(step_id, step.get("params").unwrap_or(&Value::Null), true)?;
            }
        }
    }
    Ok(())
}

/// The command id a misspelled `id`, which names no command, most likely meant: the nearest one,
/// when it is near (as [`check_keys`] matches a misspelled key).
pub(crate) fn nearest_command(id: &str) -> Option<&'static str> {
    let ids: Vec<&'static str> = command_specs().iter().map(|c| c.id).collect();
    nearest(id, &ids)
}

/// The error for `id`, which names no command: with the nearest command id, when one is near.
pub(crate) fn unknown_command(id: &str) -> crate::EngineError {
    match nearest_command(id) {
        Some(near) => crate::EngineError::UnknownCommandNear(shown_key(id), near.to_string()),
        None => crate::EngineError::UnknownCommand(shown_key(id)),
    }
}

impl crate::Session {
    /// [`check_params`], and [`check_effect_params`] for the effects `effect.setParams` edits in
    /// this session's document (not as a step of a `command.batch`: what it edits is only there
    /// once the steps before it ran).
    pub fn check_params(&self, id: &str, params: &Value) -> Result<()> {
        check_params(id, params)?;
        if let ("effect.setParams", Some(given)) = (id, params.get("params")) {
            for effect in super::effectcmd::set_params_effects(self, params) {
                check_effect_params(id, &effect, given)?;
            }
        }
        Ok(())
    }
}
