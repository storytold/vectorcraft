//! The parameter keys a command accepts, read from its params description (#1442).
//!
//! A description starts with the params object: `{x, width?: pt, name?|title?: …, sides=6, …}`, or
//! with the objects a command takes instead of one another: `{path} or {name, dataBase64}`,
//! `{name} | {names: […]}`. Each entry of an object starts with a key, or with alternatives split
//! by `|`; a `?` marks an optional one, and a `:`, `=` or `(` starts what the entry says about it. An entry that starts
//! with `…` stands for keys listed elsewhere: those of each command it names after `as` or `of`
//! (`…settings as file.new`), or, naming none, any key (the options of the format an export picks). Objects inside a value aren't
//! described key by key, so only the top level is checked.
//!
//! Callers outside the app (the control channel, MCP, the CLI) are answered with a parameter error
//! for a key the description doesn't name ([`check_params`]): a misspelt `widht` no longer runs the
//! command with its default. So are the `params` of an effect (#1592): `effect.apply` and
//! `effect.setParams` answer a key the effect doesn't take, or a value of another type than its
//! default, with a parameter error ([`check_effect_params`]). [`Session::execute`] itself takes
//! what it is given, so the journal of an older version still replays, except for an effect's
//! `color` that the effect can't read and a Levels or Curves `channel` that the effect doesn't
//! have, which `effect.apply` and `effect.setParams` refuse themselves ([`check_effect_color`],
//! [`check_effect_channel`]).

use serde_json::Value;

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
fn object_body(doc: &str) -> std::result::Result<(&str, &str), String> {
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
fn split_top(s: &str, sep: char) -> Vec<&str> {
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

fn is_key(s: &str) -> bool {
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

/// Does the description `doc` name `key` as a parameter (its own keys, and those of the commands
/// its `…` entries name)?
pub(crate) fn takes_key(doc: &'static str, key: &str) -> bool {
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

/// The parameter error of command `id` for `key`, which isn't one of `keys`: with the nearest of
/// them, or all of them.
pub(crate) fn unknown(id: &str, key: &str, keys: &[&str]) -> crate::EngineError {
    bad(
        id,
        match nearest(key, keys) {
            Some(near) => format!("unknown parameter `{key}`; did you mean `{near}`?"),
            None if keys.is_empty() => format!("unknown parameter `{key}`; the command takes no parameters"),
            None => format!("unknown parameter `{key}`; the command takes: {}", keys.join(", ")),
        },
    )
}

/// `params` of built-in effect `effect`, as command `id` was given them (#1592): only keys the
/// effect takes (those of its dialog defaults and of its params description), each of its
/// default's type, a `color` the effect can draw ([`check_effect_color`]) and a Levels or Curves
/// `channel` the effect has ([`check_effect_channel`]). Numbers out of range pass: the effect
/// clamps them when it draws. An effect plug-in resolves its own against its manifest, and an
/// unknown effect is the command's to report.
pub fn check_effect_params(id: &str, effect: &str, params: &Value) -> Result<()> {
    let Value::Object(given) = params else { return Ok(()) };
    if effect.starts_with(vectorcraft_plugins::effect::PREFIX) {
        return Ok(());
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
    for (key, v) in given {
        if !keys.contains(&key.as_str()) {
            return Err(unknown(id, key, &keys));
        }
        match defaults.and_then(|d| d.get(key)) {
            Some(Value::Number(_)) if !v.is_number() => return Err(bad(id, format!("`{key}` must be a number"))),
            Some(Value::Bool(_)) if !v.is_boolean() => return Err(bad(id, format!("`{key}` must be true or false"))),
            // (A colour is also taken as `[r, g, b]` or as a colour object.)
            Some(Value::String(_)) if v.is_number() || v.is_boolean() => return Err(bad(id, format!("`{key}` must be a string"))),
            _ => {}
        }
    }
    check_effect_color(id, effect, params)?;
    check_effect_channel(id, effect, params)
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
    if effect.starts_with(vectorcraft_plugins::effect::PREFIX) {
        return Ok(());
    }
    let takes_color = vectorcraft_render::effects::effect_info(effect).is_some_and(|info| info.defaults.get(key).is_some_and(Value::is_string));
    if takes_color && !is_effect_color(v) {
        return Err(bad(id, format!("`{key}` must be a #rrggbb color")));
    }
    Ok(())
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
    "object.setLiveShape",
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
    "stroke.set",
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

/// [`check_keys`] for an engine command, the steps of a `command.batch` included, and
/// [`check_effect_params`] for the effect `effect.apply` adds. A command the engine doesn't have
/// passes: running it reports that.
pub fn check_params(id: &str, params: &Value) -> Result<()> {
    let Some(spec) = find_command(id) else { return Ok(()) };
    if let ("effect.apply" | "appearance.addEffect", Some(effect), Some(given)) =
        (id, params.get("effect").or_else(|| params.get("id")).and_then(Value::as_str), params.get("params"))
    {
        check_effect_params(id, effect, given)?;
    }
    if UNCHECKED.contains(&id) {
        return Ok(());
    }
    check_keys(id, spec.params, params)?;
    if id == "command.batch" {
        for step in params.get("commands").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(step_id), Some(step_params)) = (step.get("command").and_then(Value::as_str), step.get("params")) {
                check_params(step_id, step_params)?;
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
        Some(near) => crate::EngineError::UnknownCommandNear(id.to_string(), near.to_string()),
        None => crate::EngineError::UnknownCommand(id.to_string()),
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
