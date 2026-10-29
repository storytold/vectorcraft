//! File → Document Setup (`document.setup`): units, bleed, the transparency grid and simulated
//! paper, output options and the document's type options.

use serde_json::{Map, Value, json};
use vectorcraft_doc::setup::{DEFAULT_FLATTENER, GRID_COLOR_PRESETS, LANGUAGES, MAX_BLEED};
use vectorcraft_doc::{Background, CharPosition, CharStyle, DocSetup, Document, ExportText, GridSize, NodeKind, Quotes, ScriptMetrics};

use super::flatten::{FlattenOptions, FlattenerPreset};

use super::{CommandSpec, Result, Session, bad, cmd, color_value, has_doc};

const C: &str = "document.setup";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "document.setup",
        "Document Setup",
        [],
        None,
        "{units?: \"Points\"|\"Inches\"|\"Millimeters\"|…, bleed?: pt (all sides)|[top, bottom, left, right]|{top?, bottom?, left?, right?} (0–72 pt), outlineImages?: bool (show images in Outline mode), highlightSubstitutedGlyphs?: bool (Highlight Substituted Fonts is a preference: prefs.set highlightSubstitutedFonts; a highlightSubstitutedFonts key is ignored and the reply carries a warning),gridSize?: \"small\"|\"medium\"|\"large\", gridColors?: \"Light\"|\"Medium\"|\"Dark\"|\"Red\"|\"Orange\"|\"Green\"|\"Blue\"|\"Purple\"|[colour, colour] (transparency grid; the first is also the paper colour), simulatePaper?: bool, flattenerPreset?: \"High Resolution\"|\"Medium Resolution\"|\"Low Resolution\"|a saved preset (flattener.presets.list), discardWhiteOverprint?: bool, language?: \"English: USA\"|\"German\"|\"French\"|… (also picks its quotes), quotes?: {double?: \"“”\", single?: \"‘’\"} (beside a language: only if they differ from the current ones), typographersQuotes?: bool (typed straight quotes become the document's quotes), superscript?: {size?: %, position?: %}, subscript?: {size?: %, position?: %}, smallCapsSize?: %, exportText?: \"editable\"|\"appearance\" (SVG text as text or as outlines), backgroundContents?: \"transparent\"|\"white\" (white: raster exports are white behind the art), …the keys of its own reply (it can be passed back; the values it only reports are ignored)} change the document setup in one undo step; no params → the current setup",
        has_doc,
        setup
    )]
}

/// The setup as `document.setup` reports it (and accepts it back).
pub(crate) fn setup_json(d: &Document) -> Value {
    let s = &d.setup;
    let pair = |p: [char; 2]| p.iter().collect::<String>();
    json!({
        "units": d.units.label(),
        "bleed": s.bleed,
        "outlineImages": s.outline_images,
        "highlightSubstitutedGlyphs": s.highlight_substituted_glyphs,
        "gridSize": s.grid_size.id(),
        "gridColors": s.grid_colors.map(|c| c.to_hex()),
        "gridColorsName": s.grid_colors_name(),
        "simulatePaper": s.simulate_paper,
        "flattenerPreset": s.flattener(),
        "discardWhiteOverprint": s.discard_white_overprint,
        "language": s.language,
        "quotes": {"double": pair(s.quotes.double), "single": pair(s.quotes.single)},
        "typographersQuotes": s.typographers_quotes,
        "superscript": s.superscript,
        "subscript": s.subscript,
        "smallCapsSize": s.small_caps_size,
        "exportText": s.export_text.id(),
        "backgroundContents": s.background.id(),
    })
}

fn setup(s: &mut Session, p: &Value) -> Result<Value> {
    let empty = Map::new();
    let o = match p {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err(bad(C, "params must be an object")),
    };
    let st = s.doc()?;
    if o.is_empty() {
        return Ok(setup_json(&st.doc));
    }
    let mut new = st.doc.setup.clone();
    let mut units = st.doc.units;
    // The language goes first: it picks its quotes, which `quotes` may then override.
    let current = st.doc.setup.quotes;
    for (k, v) in o.iter().filter(|(k, _)| *k == "language").chain(o.iter().filter(|(k, _)| *k != "language")) {
        // With a language, unchanged quotes (a report fed back) don't undo the language's quotes.
        if k == "quotes" && o.contains_key("language") && quotes_param(v, current)? == current {
            continue;
        }
        apply(&mut new, &mut units, k, v, &s.prefs.flattener_presets)?;
    }
    // An older build's reply passed back still applies; the moved key only gets a warning.
    let reply = |d: &Document| {
        let mut r = setup_json(d);
        if o.contains_key("highlightSubstitutedFonts")
            && let Some(m) = r.as_object_mut()
        {
            m.insert("warnings".into(), json!([MOVED_FONTS_HIGHLIGHT]));
        }
        r
    };
    let st = s.doc()?;
    if new == st.doc.setup && units == st.doc.units {
        return Ok(reply(&st.doc));
    }
    s.edit("Document Setup", |d, _| {
        let scripts_changed =
            (new.superscript, new.subscript, new.small_caps_size) != (d.setup.superscript, d.setup.subscript, d.setup.small_caps_size);
        d.setup = new;
        d.units = units;
        if scripts_changed {
            restamp_scripts(d);
        }
        Ok(())
    })?;
    Ok(reply(&s.doc()?.doc))
}

/// The warning for `highlightSubstitutedFonts`, which `document.setup` ignores.
const MOVED_FONTS_HIGHLIGHT: &str =
    "highlightSubstitutedFonts is ignored: it is a preference (Preferences › Type), set with prefs.set {key: \"highlightSubstitutedFonts\", value}";

fn flag(k: &str, v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| bad(C, format!("{k} must be true or false")))
}

fn percent(k: &str, v: &Value, min: f64, max: f64) -> Result<f64> {
    v.as_f64().filter(|x| (min..=max).contains(x)).ok_or_else(|| bad(C, format!("{k} must be a number from {min} to {max}")))
}

fn apply(s: &mut DocSetup, units: &mut vectorcraft_doc::Unit, k: &str, v: &Value, saved: &[FlattenerPreset]) -> Result<()> {
    match k {
        "units" => *units = v.as_str().and_then(vectorcraft_doc::Unit::named).ok_or_else(|| bad(C, "unknown units"))?,
        "bleed" => s.bleed = bleed_param(v, s.bleed, C)?,
        "outlineImages" => s.outline_images = flag(k, v)?,
        // Preferences › Type holds it for every document: ignored, with a warning in the reply.
        "highlightSubstitutedFonts" => {
            flag(k, v)?;
        }
        "highlightSubstitutedGlyphs" => s.highlight_substituted_glyphs = flag(k, v)?,
        "gridSize" => s.grid_size = v.as_str().and_then(GridSize::parse).ok_or_else(|| bad(C, "gridSize must be small, medium or large"))?,
        "gridColors" => s.grid_colors = grid_colors(v)?,
        "simulatePaper" => s.simulate_paper = flag(k, v)?,
        "flattenerPreset" => s.flattener_preset = flattener_preset(v, saved)?,
        "discardWhiteOverprint" => s.discard_white_overprint = flag(k, v)?,
        "language" => {
            let l = v.as_str().unwrap_or("");
            let (name, quotes) = LANGUAGES.iter().find(|(n, _)| n.eq_ignore_ascii_case(l)).ok_or_else(|| {
                bad(C, format!("unknown language `{l}` (one of {})", LANGUAGES.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")))
            })?;
            s.language = name.to_string();
            s.quotes = *quotes;
        }
        "quotes" => s.quotes = quotes_param(v, s.quotes)?,
        "typographersQuotes" => s.typographers_quotes = flag(k, v)?,
        "superscript" => s.superscript = script_param(k, v, s.superscript)?,
        "subscript" => s.subscript = script_param(k, v, s.subscript)?,
        "smallCapsSize" => s.small_caps_size = percent(k, v, 1.0, vectorcraft_doc::setup::MAX_SMALL_CAPS)?,
        "exportText" => s.export_text = v.as_str().and_then(ExportText::parse).ok_or_else(|| bad(C, "exportText must be editable or appearance"))?,
        "backgroundContents" => s.background = background_param(v, C)?,
        // Read-only values `document.setup` reports.
        "gridColorsName" | "warnings" => {}
        _ => return Err(bad(C, format!("unknown setting `{k}`"))),
    }
    Ok(())
}

/// A built-in flattener preset (by name or id) or a saved one, stored by its display name (None:
/// the default preset).
fn flattener_preset(v: &Value, saved: &[FlattenerPreset]) -> Result<Option<String>> {
    let name = v.as_str().unwrap_or("").trim();
    let builtin = FlattenOptions::PRESETS
        .into_iter()
        .filter_map(|id| Some((id, FlattenOptions::preset_label(id)?)))
        .find(|(id, label)| id.eq_ignore_ascii_case(name) || label.eq_ignore_ascii_case(name))
        .map(|(_, label)| label);
    let found = builtin.or_else(|| saved.iter().map(|p| p.name.as_str()).find(|n| n.eq_ignore_ascii_case(name))).ok_or_else(|| {
        bad(
            C,
            format!(
                "unknown flattenerPreset `{name}` (High Resolution, Medium Resolution, Low Resolution or a saved one: see flattener.presets.list)"
            ),
        )
    })?;
    Ok((found != DEFAULT_FLATTENER).then(|| found.to_string()))
}

/// `"transparent"` or `"white"`.
pub(crate) fn background_param(v: &Value, cmd: &str) -> Result<Background> {
    v.as_str().and_then(Background::parse).ok_or_else(|| bad(cmd, "backgroundContents must be transparent or white"))
}

/// `pt` (every side), `[top, bottom, left, right]` or `{top?, bottom?, left?, right?}`.
pub(crate) fn bleed_param(v: &Value, current: [f64; 4], cmd: &str) -> Result<[f64; 4]> {
    let side = |x: &Value| x.as_f64().filter(|b| (0.0..=MAX_BLEED).contains(b));
    let err = || bad(cmd, format!("bleed must be 0–{MAX_BLEED} pt: a number, [top, bottom, left, right] or {{top, bottom, left, right}}"));
    match v {
        Value::Number(_) => side(v).map(|b| [b; 4]).ok_or_else(err),
        Value::Array(a) if a.len() == 4 => {
            let mut out = [0.0; 4];
            for (o, x) in out.iter_mut().zip(a) {
                *o = side(x).ok_or_else(err)?;
            }
            Ok(out)
        }
        Value::Object(o) => {
            let mut out = current;
            for (k, x) in o {
                let i = ["top", "bottom", "left", "right"].iter().position(|s| s == k).ok_or_else(err)?;
                out[i] = side(x).ok_or_else(err)?;
            }
            Ok(out)
        }
        _ => Err(err()),
    }
}

fn grid_colors(v: &Value) -> Result<[vectorcraft_color::Color; 2]> {
    if let Some(name) = v.as_str() {
        return GRID_COLOR_PRESETS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, c)| *c).ok_or_else(|| {
            bad(C, format!("unknown grid colours `{name}` (one of {})", GRID_COLOR_PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")))
        });
    }
    match v.as_array().map(|a| a.iter().map(color_value).collect::<Option<Vec<_>>>()) {
        Some(Some(c)) if c.len() == 2 => Ok([c[0], c[1]]),
        _ => Err(bad(C, "gridColors must be a preset name or two colours")),
    }
}

/// A quote pair: a two-character string or two one-character strings.
fn quote_pair(v: &Value) -> Option<[char; 2]> {
    let chars: Vec<char> = match v {
        Value::String(s) => s.chars().collect(),
        Value::Array(a) => a.iter().map(|x| x.as_str().and_then(|s| s.chars().next().filter(|_| s.chars().count() == 1))).collect::<Option<_>>()?,
        _ => return None,
    };
    match chars[..] {
        [a, b] if !a.is_control() && !b.is_control() && !a.is_whitespace() && !b.is_whitespace() => Some([a, b]),
        _ => None,
    }
}

fn quotes_param(v: &Value, mut q: Quotes) -> Result<Quotes> {
    let o = v.as_object().ok_or_else(|| bad(C, "quotes must be {double?, single?}"))?;
    for (k, x) in o {
        let pair = quote_pair(x).ok_or_else(|| bad(C, format!("quotes.{k} must be an opening and a closing quote, e.g. \"“”\"")))?;
        match k.as_str() {
            "double" => q.double = pair,
            "single" => q.single = pair,
            _ => return Err(bad(C, format!("unknown quotes key `{k}` (double, single)"))),
        }
    }
    Ok(q)
}

fn script_param(k: &str, v: &Value, mut m: ScriptMetrics) -> Result<ScriptMetrics> {
    let o = v.as_object().ok_or_else(|| bad(C, format!("{k} must be {{size?, position?}}")))?;
    for (key, x) in o {
        match key.as_str() {
            "size" => m.size = percent(&format!("{k}.size"), x, 1.0, vectorcraft_doc::setup::MAX_SCRIPT)?,
            "position" => m.position = percent(&format!("{k}.position"), x, -vectorcraft_doc::setup::MAX_SCRIPT, vectorcraft_doc::setup::MAX_SCRIPT)?,
            _ => return Err(bad(C, format!("unknown {k} key `{key}` (size, position)"))),
        }
    }
    Ok(m)
}

/// `position?: "normal"|"superscript"|"subscript"` and `smallCaps?: bool` from a character
/// command's params, in the document's Document Setup proportions.
pub(crate) fn script_params(p: &Value, setup: &DocSetup, cmd: &str) -> Result<(Option<CharPosition>, Option<Option<f64>>)> {
    let position = match p.get("position").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(match v.as_str().map(str::to_ascii_lowercase).as_deref() {
            Some("normal") => CharPosition::Normal,
            Some("superscript") => CharPosition::Superscript(setup.superscript),
            Some("subscript") => CharPosition::Subscript(setup.subscript),
            _ => return Err(bad(cmd, "position must be normal, superscript or subscript")),
        }),
    };
    let small_caps = match p.get("smallCaps").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(cmd, "smallCaps must be true or false"))?.then_some(setup.small_caps_size)),
    };
    Ok((position, small_caps))
}

/// A superscript or subscript in `setup`'s proportions.
pub(crate) fn position_in(setup: &DocSetup, p: CharPosition) -> CharPosition {
    match p {
        CharPosition::Normal => CharPosition::Normal,
        CharPosition::Superscript(_) => CharPosition::Superscript(setup.superscript),
        CharPosition::Subscript(_) => CharPosition::Subscript(setup.subscript),
    }
}

/// After the superscript, subscript or small caps proportions changed: every run (and character
/// style) using them takes the new ones.
fn restamp_scripts(d: &mut Document) {
    let uses = |st: &CharStyle| st.position != CharPosition::Normal || st.small_caps.is_some();
    let mut ids = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind
            && t.runs.iter().any(|r| uses(&r.style))
        {
            ids.push(n.id);
        }
    });
    let setup = d.setup.clone();
    for id in ids {
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            for st in t.runs.iter_mut().map(|r| &mut r.style).filter(|st| uses(st)) {
                st.position = position_in(&setup, st.position);
                st.small_caps = st.small_caps.map(|_| setup.small_caps_size);
            }
            super::typecmd::refresh_bounds(t);
        }
    }
    // Character styles store the attributes they set by their serialized names.
    for def in &mut d.char_styles {
        if let Some(v) = def.attrs.get_mut("position")
            && let Ok(p) = serde_json::from_value::<CharPosition>(v.clone())
        {
            *v = serde_json::to_value(position_in(&setup, p)).unwrap_or(Value::Null);
        }
        if let Some(v) = def.attrs.get_mut("small_caps").filter(|v| v.is_number()) {
            *v = json!(setup.small_caps_size);
        }
    }
}
