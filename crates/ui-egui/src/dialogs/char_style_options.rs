//! Character Style Options fields shared by editing and creating a character style.

use serde_json::{Map, Value, json};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub const KIND: &str = "charStyleOptions";
const BASIC: &[&str] = &[
    "font_family",
    "font_style",
    "size",
    "leading",
    "kerning",
    "tracking",
    "all_caps",
    "small_caps",
    "position",
    "underline",
    "strikethrough",
    "standardVerticalRoman",
    "charAlign",
];
const NUMBERS: &[&str] = &["size", "leading", "kerning", "tracking"];
/// The on, off or unset checkboxes.
const SWITCHES: &[&str] = &["underline", "strikethrough", "standardVerticalRoman"];
/// The attributes of Advanced Character Formats, in the order the section shows them.
const ADVANCED: &[&str] = &["h_scale", "v_scale", "baseline_shift", "rotation", "language"];
/// The number fields of Advanced Character Formats.
const ADVANCED_NUMBERS: &[&str] = &["h_scale", "v_scale", "baseline_shift", "rotation"];
/// The attributes of Character Color.
const COLOR: &[&str] = &["fill", "stroke", "stroke_width", "overprint_fill", "overprint_stroke"];
/// The fields of Character Color: the fill and stroke paints, their tints in percent, the stroke
/// weight in points and the two overprint checkboxes.
const COLOR_FIELDS: &[&str] = &["fill", "fill_tint", "stroke", "stroke_tint", "stroke_width", "overprint_fill", "overprint_stroke"];
/// The on, off or unset checkboxes of OpenType Features and the feature tag each one sets.
const OT_SWITCHES: &[(&str, &str)] = &[
    ("ot_liga", "liga"),
    ("ot_calt", "calt"),
    ("ot_dlig", "dlig"),
    ("ot_swsh", "swsh"),
    ("ot_salt", "salt"),
    ("ot_titl", "titl"),
    ("ot_ordn", "ordn"),
    ("ot_frac", "frac"),
];
/// The feature tags the Figure menu sets.
const FIGURE_TAGS: [&str; 4] = ["tnum", "onum", "pnum", "lnum"];
/// The values of the Figure field, in the order of the OpenType panel's Figure menu.
const FIGURE_VALUES: [&str; 5] = ["default", "tabularLining", "proportionalLining", "proportionalOldstyle", "tabularOldstyle"];
/// The values of the Position field, in the order of the OpenType panel's Position menu.
const POSITION_VALUES: [&str; 5] = ["default", "superscript", "subscript", "numerator", "denominator"];
/// The fields of OpenType Features.
const OT_FIELDS: &[&str] =
    &["ot_figure", "ot_position", "ot_liga", "ot_calt", "ot_dlig", "ot_swsh", "ot_salt", "ot_titl", "ot_ordn", "ot_frac", "ot_sets"];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Character Style Options").into(),
    body,
    confirm,
    preview: true,
    inputs: |_| {
        &[
            "name",
            "font_family",
            "font_style",
            "size",
            "leading",
            "kerning",
            "tracking",
            "case",
            "position",
            "underline",
            "strikethrough",
            "standardVerticalRoman",
            "charAlign",
            "h_scale",
            "v_scale",
            "baseline_shift",
            "rotation",
            "language",
            "fill",
            "fill_tint",
            "stroke",
            "stroke_tint",
            "stroke_width",
            "overprint_fill",
            "overprint_stroke",
            "ot_figure",
            "ot_position",
            "ot_liga",
            "ot_calt",
            "ot_dlig",
            "ot_swsh",
            "ot_salt",
            "ot_titl",
            "ot_ordn",
            "ot_frac",
            "ot_sets",
            "section",
        ]
    },
    min_width: 650.0,
    ..DialogSpec::FORM
};

pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    crate::panels::character::end_typing(app);
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let def = doc.char_styles.iter().find(|s| s.name == name).ok_or_else(|| format!("no character style `{name}`"))?;
    let mut fields = fields_from_attrs(&def.attrs);
    fields.insert("name".into(), json!(name));
    fields.insert("__style".into(), json!(name));
    fields.insert("__attrs".into(), json!(def.attrs));
    fields.insert("__smallCapsSize".into(), json!(doc.setup.small_caps_size));
    fields.insert("__superscript".into(), json!(doc.setup.superscript));
    fields.insert("__subscript".into(), json!(doc.setup.subscript));
    fields.insert("__swatches".into(), swatch_choices(doc));
    fields.insert("section".into(), json!("general"));
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({"dialog": KIND}))
}

pub(crate) fn fields_from_attrs(attrs: &Map<String, Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    for key in
        ["font_family", "font_style", "size", "leading", "kerning", "tracking", "underline", "strikethrough", "standardVerticalRoman", "charAlign"]
    {
        let value = attrs.get(key).cloned().unwrap_or(Value::Null);
        let shown = if SWITCHES.contains(&key) {
            value
        } else if value.is_null() {
            if attrs.contains_key(key) && ["leading", "kerning"].contains(&key) { json!("Auto") } else { json!("") }
        } else {
            value
        };
        fields.insert(key.into(), shown);
    }
    fields.insert("case".into(), case_from_attrs(attrs).map_or(Value::Null, Value::from));
    fields.insert("position".into(), attrs.get("position").and_then(|v| v.get("kind")).cloned().unwrap_or(Value::Null));
    for key in ADVANCED_NUMBERS {
        fields.insert((*key).into(), attrs.get(*key).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")));
    }
    fields.insert("language".into(), language_field(attrs.get("language")));
    for key in ["fill", "stroke"] {
        let paint = attrs.get(key).cloned().unwrap_or(Value::Null);
        fields.insert(format!("{key}_tint"), tint_field(&paint));
        fields.insert(key.into(), paint);
    }
    fields.insert("stroke_width".into(), attrs.get("stroke_width").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")));
    for key in ["overprint_fill", "overprint_stroke"] {
        fields.insert(key.into(), attrs.get(key).cloned().unwrap_or(Value::Null));
    }
    fields.extend(opentype_fields(attrs.get("features")));
    fields
}

/// The Tint field for a paint: its tint in percent when it links to a swatch, else empty.
fn tint_field(paint: &Value) -> Value {
    if paint.get("swatch").is_some_and(Value::is_string) {
        json!(vectorcraft_color::tint_percent(paint.get("tint").and_then(Value::as_f64).unwrap_or(1.0) as f32))
    } else {
        json!("")
    }
}

/// The swatches Character Color offers, as `[name, paint]` pairs in the Swatches panel's order, the
/// built-in Registration swatch first: a global color or a tint of one as a paint linked to it, any
/// other solid color as that color.
pub(crate) fn swatch_choices(doc: &vectorcraft_doc::Document) -> Value {
    let registration = vectorcraft_color::swatch::registration();
    let builtin = (!doc.swatches_iter().any(|w| w.name == registration.name)).then_some(registration);
    let choices = builtin.into_iter().chain(doc.swatches_iter()).filter_map(|w| {
        let paint = match (doc.swatch_link(&w.name), &w.paint) {
            (Some((base, tint)), _) => doc.tint_paint(&base, tint)?,
            (None, vectorcraft_color::Paint::Solid { .. }) => w.paint.clone(),
            _ => return None,
        };
        Some(json!([w.name, paint]))
    });
    Value::Array(choices.collect())
}

/// A feature tag without its `-` or `+` prefix and its `=N` value.
fn bare(tag: &str) -> &str {
    let tag = tag.trim_start_matches(['-', '+']);
    tag.split_once('=').map_or(tag, |(name, _)| name)
}

/// The OpenType Features fields for a style's `features`: each control is unset unless the list
/// names one of its tags, on or off. Figure is also unset when the list turns one of its tags on
/// or off against every Figure menu entry, for example `pnum` alone.
fn opentype_fields(features: Option<&Value>) -> Map<String, Value> {
    let tags: Vec<&str> = features.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let names = |set: &[&str]| tags.iter().any(|t| set.contains(&bare(t)));
    let f = vectorcraft_text::OtFeatures::from_tags(tags.iter().copied());
    let mut fields = Map::new();
    for (field, tag) in OT_SWITCHES {
        let last = tags.iter().rev().find(|t| bare(t) == *tag);
        fields.insert((*field).into(), last.map_or(Value::Null, |t| json!(!t.starts_with('-'))));
    }
    let last_on = |tag: &str| tags.iter().rev().find(|t| bare(t) == tag).map(|t| !t.starts_with('-'));
    let figure = names(&FIGURE_TAGS)
        .then(|| FIGURE_VALUES.get(crate::panels::opentype::figure(&f)))
        .flatten()
        .filter(|v| figure_tags(v).iter().all(|t| last_on(bare(t)).is_none_or(|on| on != t.starts_with('-'))));
    fields.insert("ot_figure".into(), figure.map_or(Value::Null, |v| json!(v)));
    let position =
        names(&crate::panels::opentype::POSITION_TAGS).then(|| crate::panels::opentype::position(&f).and_then(|i| POSITION_VALUES.get(i))).flatten();
    fields.insert("ot_position".into(), position.map_or(Value::Null, |v| json!(v)));
    let sets: Vec<String> = vectorcraft_text::EXTRA_FEATURES[..20]
        .iter()
        .filter_map(|set| tags.iter().rev().find(|t| bare(t) == *set).map(|t| if t.starts_with('-') { format!("-{set}") } else { set.to_string() }))
        .collect();
    fields.insert("ot_sets".into(), if sets.is_empty() { Value::Null } else { json!(sets) });
    fields
}

/// The Language field for a style's `language` attribute: unset for none, the default language's
/// name for an explicit default (`null`), a listed tag's name, or a tag not in the list as it is.
fn language_field(value: Option<&Value>) -> Value {
    match value {
        None => Value::Null,
        Some(Value::Null) => vectorcraft_doc::setup::char_language_name(None).map_or(Value::Null, Value::from),
        Some(Value::String(tag)) => json!(vectorcraft_doc::setup::char_language_name(Some(tag)).unwrap_or(tag)),
        Some(other) => other.clone(),
    }
}

fn case_from_attrs(attrs: &Map<String, Value>) -> Option<&'static str> {
    if attrs.get("all_caps") == Some(&json!(true)) {
        Some("allCaps")
    } else if attrs.get("small_caps").is_some_and(|v| !v.is_null()) {
        Some("smallCaps")
    } else if attrs.contains_key("all_caps") || attrs.contains_key("small_caps") {
        Some("normal")
    } else {
        None
    }
}

fn optional_number(d: &Dialog, key: &str) -> Result<Option<Value>, String> {
    let label = match key {
        "size" => tl!("Size"),
        "leading" => tl!("Leading"),
        "kerning" => tl!("Kerning"),
        "tracking" => tl!("Tracking"),
        "h_scale" => tl!("Horizontal Scale"),
        "v_scale" => tl!("Vertical Scale"),
        "baseline_shift" => tl!("Baseline Shift"),
        "rotation" => tl!("Character Rotation"),
        "fill_tint" | "stroke_tint" => tl!("Tint"),
        "stroke_width" => tl!("Weight"),
        _ => return Err("unknown character style number field".into()),
    };
    match d.fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("auto") && ["leading", "kerning"].contains(&key) => Ok(Some(Value::Null)),
        Some(v) => {
            let n = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("{label} must be a number"))?;
            // Character Style fields use the Character panel's visible ranges.
            let max = match key {
                "size" => 1296.0,
                "leading" => 5000.0,
                _ => vectorcraft_doc::MAX_COORD,
            };
            if ["size", "leading"].contains(&key) && n < 0.1 {
                return Err(format!("{label} must be a number of at least 0.1"));
            }
            if ["size", "leading"].contains(&key) && n > max {
                return Err(format!("{label} must be a number at most {max}"));
            }
            if ["h_scale", "v_scale"].contains(&key) && !(1.0..=10000.0).contains(&n) {
                return Err(format!("{label} must be a number from 1 to 10000"));
            }
            if key == "baseline_shift" && !(-1296.0..=1296.0).contains(&n) {
                return Err(format!("{label} must be a number from -1296 to 1296"));
            }
            // Weight from 0 to the document limit, a tint from 0% to 100%.
            let (min, max) = match key {
                "stroke_width" => (0.0, vectorcraft_doc::MAX_COORD),
                "fill_tint" | "stroke_tint" => (0.0, 100.0),
                _ => (0.0, 0.0),
            };
            if ["stroke_width", "fill_tint", "stroke_tint"].contains(&key) && !(min..=max).contains(&n) {
                return Err(format!("{label} must be a number from {min} to {max}"));
            }
            // An angle from -180 to 180 degrees, as text.setFormat stores it.
            let n = if key == "rotation" { (n + 180.0).rem_euclid(360.0) - 180.0 } else { n };
            Ok(Some(json!(n)))
        }
    }
}

pub(crate) fn edited_attrs(d: &Dialog) -> Result<Map<String, Value>, String> {
    let original = d.fields.get("__attrs").and_then(Value::as_object).cloned().unwrap_or_default();
    let original_fields = fields_from_attrs(&original);
    let reset = d.bool("__resetBasic");
    let mut attrs = original.clone();
    for key in BASIC {
        attrs.remove(*key);
    }
    for key in ["font_family", "font_style"] {
        if let Some(v) = d.fields.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            attrs.insert(key.into(), json!(v.trim()));
        }
    }
    if reset || ["font_family", "font_style"].iter().any(|key| d.fields.get(*key) != original_fields.get(*key)) {
        attrs.remove("font_version");
        attrs.remove("fontAxes");
    }
    for key in NUMBERS {
        // An unchanged field keeps the original value below, unchecked (see `advanced_attrs`).
        if !reset && d.fields.get(*key) == original_fields.get(*key) {
            continue;
        }
        if let Some(v) = optional_number(d, key)? {
            attrs.insert((*key).into(), v);
        }
    }
    for key in SWITCHES {
        if let Some(v) = d.fields.get(*key).and_then(Value::as_bool) {
            attrs.insert((*key).into(), json!(v));
        }
    }
    if reset || d.fields.get("case").and_then(Value::as_str) != case_from_attrs(&original) {
        match d.fields.get("case").and_then(Value::as_str) {
            Some("allCaps") => {
                attrs.insert("all_caps".into(), json!(true));
                attrs.insert("small_caps".into(), Value::Null);
            }
            Some("smallCaps") => {
                attrs.insert("all_caps".into(), json!(false));
                attrs.insert(
                    "small_caps".into(),
                    original
                        .get("small_caps")
                        .filter(|v| v.is_number())
                        .cloned()
                        .unwrap_or_else(|| d.fields.get("__smallCapsSize").cloned().unwrap_or_else(|| json!(70.0))),
                );
            }
            Some("normal") => {
                attrs.insert("all_caps".into(), json!(false));
                attrs.insert("small_caps".into(), Value::Null);
            }
            _ => {}
        }
    }
    if let Some(position) = d.fields.get("position").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        let value = if position == "normal" {
            json!({"kind": "normal"})
        } else {
            original.get("position").filter(|v| v.get("kind").and_then(Value::as_str) == Some(position)).cloned().unwrap_or_else(|| {
                let metrics = d.fields.get(if position == "superscript" { "__superscript" } else { "__subscript" });
                json!({"kind": position, "size": metrics.and_then(|m| m.get("size")).cloned().unwrap_or(json!(58.3)),
                        "position": metrics.and_then(|m| m.get("position")).cloned().unwrap_or(json!(33.3))})
            })
        };
        attrs.insert("position".into(), value);
    }
    if let Some(align) = d.fields.get("charAlign").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        attrs.insert("charAlign".into(), json!(align));
    }
    if !reset {
        for key in BASIC {
            let field = if ["all_caps", "small_caps"].contains(key) { "case" } else { key };
            if d.fields.get(field) == original_fields.get(field) {
                attrs.remove(*key);
                if let Some(value) = original.get(*key) {
                    attrs.insert((*key).into(), value.clone());
                }
            }
        }
    }
    advanced_attrs(d, &original, &original_fields, &mut attrs)?;
    color_attrs(d, &original, &original_fields, &mut attrs)?;
    opentype_attrs(d, &original, &original_fields, &mut attrs);
    Ok(attrs)
}

/// Write the Character Color fields of `d` into `attrs`: an unset paint, weight or checkbox
/// leaves its attribute unset, and a field still as it opened keeps the original value, a
/// gradient or pattern paint included.
fn color_attrs(
    d: &Dialog,
    original: &Map<String, Value>,
    original_fields: &Map<String, Value>,
    attrs: &mut Map<String, Value>,
) -> Result<(), String> {
    let reset = d.bool("__resetColor");
    for key in COLOR {
        attrs.remove(*key);
    }
    let unchanged = |key: &str| !reset && d.fields.get(key) == original_fields.get(key);
    for key in ["fill", "stroke"] {
        let tint_key = format!("{key}_tint");
        if unchanged(key) && unchanged(&tint_key) {
            if let Some(value) = original.get(key) {
                attrs.insert(key.into(), value.clone());
            }
            continue;
        }
        let Some(mut paint) = d.fields.get(key).filter(|v| v.is_object()).cloned() else { continue };
        if paint.get("swatch").is_some_and(Value::is_string)
            && let Some(Value::Number(tint)) = optional_number(d, &tint_key)?
            && let Some(tint) = tint.as_f64()
        {
            paint["tint"] = json!(tint / 100.0);
        }
        attrs.insert(key.into(), paint);
    }
    if unchanged("stroke_width") {
        if let Some(value) = original.get("stroke_width") {
            attrs.insert("stroke_width".into(), value.clone());
        }
    } else if let Some(v) = optional_number(d, "stroke_width")? {
        attrs.insert("stroke_width".into(), v);
    }
    for key in ["overprint_fill", "overprint_stroke"] {
        let value = if unchanged(key) { original.get(key).cloned() } else { d.fields.get(key).filter(|v| v.is_boolean()).cloned() };
        if let Some(value) = value {
            attrs.insert(key.into(), value);
        }
    }
    Ok(())
}

/// The tags a Figure field value sets: its figure features on and the others off.
fn figure_tags(value: &str) -> &'static [&'static str] {
    match value {
        "tabularLining" => &["tnum", "lnum", "-onum", "-pnum"],
        "proportionalLining" => &["pnum", "lnum", "-tnum", "-onum"],
        "proportionalOldstyle" => &["onum", "pnum", "-tnum", "-lnum"],
        "tabularOldstyle" => &["onum", "tnum", "-pnum", "-lnum"],
        "default" => &["-tnum", "-onum", "-pnum", "-lnum"],
        _ => &[],
    }
}

/// Write the OpenType Features fields of `d` into `attrs`. When every field is still as it
/// opened, the original `features` list stays as it is. Otherwise each changed control replaces
/// the tags it sets in the original list, which keeps every other tag, and an unset control
/// removes them; a list left empty leaves `features` unset.
fn opentype_attrs(d: &Dialog, original: &Map<String, Value>, original_fields: &Map<String, Value>, attrs: &mut Map<String, Value>) {
    let reset = d.bool("__resetOpenType");
    attrs.remove("features");
    let unchanged = |key: &str| !reset && d.fields.get(key) == original_fields.get(key);
    if OT_FIELDS.iter().all(|key| unchanged(key)) {
        if let Some(value) = original.get("features") {
            attrs.insert("features".into(), value.clone());
        }
        return;
    }
    let mut tags: Vec<String> = if reset {
        vec![]
    } else {
        original.get("features").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
    };
    for (field, tag) in OT_SWITCHES {
        if !unchanged(field) {
            tags.retain(|t| bare(t) != *tag);
            if let Some(on) = d.fields.get(*field).and_then(Value::as_bool) {
                tags.push(if on { (*tag).to_string() } else { format!("-{tag}") });
            }
        }
    }
    if !unchanged("ot_figure") {
        tags.retain(|t| !FIGURE_TAGS.contains(&bare(t)));
        tags.extend(figure_tags(&d.str("ot_figure")).iter().map(|t| t.to_string()));
    }
    if !unchanged("ot_position") {
        let position_tags = crate::panels::opentype::POSITION_TAGS;
        tags.retain(|t| !position_tags.contains(&bare(t)));
        if let Some(i) = POSITION_VALUES.iter().position(|v| *v == d.str("ot_position")) {
            for (k, tag) in position_tags.iter().enumerate() {
                tags.push(if k + 1 == i { (*tag).to_string() } else { format!("-{tag}") });
            }
        }
    }
    if !unchanged("ot_sets") {
        let sets = &vectorcraft_text::EXTRA_FEATURES[..20];
        // A set still on keeps the value its tag gives (`ss01=2`).
        let before: Vec<String> = tags.iter().filter(|t| sets.contains(&bare(t)) && !t.starts_with('-')).cloned().collect();
        tags.retain(|t| !sets.contains(&bare(t)));
        let listed = d.fields.get("ot_sets").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        tags.extend(listed.filter(|t| sets.contains(&bare(t))).map(|t| {
            let kept = before.iter().rev().find(|b| !t.starts_with('-') && bare(b) == bare(t));
            kept.map_or_else(|| t.to_string(), Clone::clone)
        }));
    }
    if !tags.is_empty() {
        attrs.insert("features".into(), json!(tags));
    }
}

/// Write the Advanced Character Formats fields of `d` into `attrs`: a filled field sets its
/// attribute, an empty one leaves it unset, and a field still as it opened keeps the original
/// value, an explicit default or a language tag not in the list included.
fn advanced_attrs(
    d: &Dialog,
    original: &Map<String, Value>,
    original_fields: &Map<String, Value>,
    attrs: &mut Map<String, Value>,
) -> Result<(), String> {
    let reset = d.bool("__resetAdvanced");
    for key in ADVANCED {
        attrs.remove(*key);
    }
    // A field still as it opened keeps its original value below without a check.
    let unchanged = |key: &str| !reset && d.fields.get(key) == original_fields.get(key);
    for key in ADVANCED_NUMBERS {
        if unchanged(key) {
            continue;
        }
        if let Some(v) = optional_number(d, key)? {
            attrs.insert((*key).into(), v);
        }
    }
    if !unchanged("language")
        && let Some(language) = d.fields.get("language").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
    {
        let value = match vectorcraft_doc::setup::char_language(language) {
            Ok(tag) => tag.map_or(Value::Null, Value::from),
            // A tag not in the list goes to the command as it is: charStyle.edit keeps it only
            // when the style already has it.
            Err(_) => json!(language),
        };
        attrs.insert("language".into(), value);
    }
    for key in ADVANCED {
        if unchanged(key)
            && let Some(value) = original.get(*key)
        {
            attrs.insert((*key).into(), value.clone());
        }
    }
    Ok(())
}

pub(crate) fn reset_basic(d: &mut Dialog) {
    d.fields.insert("__resetBasic".into(), json!(true));
    for key in [
        "font_family",
        "font_style",
        "size",
        "leading",
        "kerning",
        "tracking",
        "case",
        "position",
        "underline",
        "strikethrough",
        "standardVerticalRoman",
        "charAlign",
    ] {
        d.fields.insert(
            key.into(),
            if ["font_family", "font_style", "size", "leading", "kerning", "tracking"].contains(&key) { json!("") } else { Value::Null },
        );
    }
}

/// Reset Panel in Advanced Character Formats: its five attributes become unset.
pub(crate) fn reset_advanced(d: &mut Dialog) {
    d.fields.insert("__resetAdvanced".into(), json!(true));
    for key in ADVANCED_NUMBERS {
        d.fields.insert((*key).into(), json!(""));
    }
    d.fields.insert("language".into(), Value::Null);
}

/// Reset Panel in Character Color: its paints, tints, weight and overprint checkboxes become unset.
pub(crate) fn reset_color(d: &mut Dialog) {
    d.fields.insert("__resetColor".into(), json!(true));
    for key in COLOR_FIELDS {
        let unset = if ["fill_tint", "stroke_tint", "stroke_width"].contains(key) { json!("") } else { Value::Null };
        d.fields.insert((*key).into(), unset);
    }
}

/// Reset Panel in OpenType Features: every control becomes unset, and the style's `features`
/// with it.
pub(crate) fn reset_opentype(d: &mut Dialog) {
    d.fields.insert("__resetOpenType".into(), json!(true));
    for key in OT_FIELDS {
        d.fields.insert((*key).into(), Value::Null);
    }
}

/// Whether the section `d` shows has a Reset Panel: every section but General.
pub(crate) fn can_reset(d: &Dialog) -> bool {
    ["basic", "advanced", "color", "opentype"].contains(&d.str("section").as_str())
}

/// Reset Panel: clear the settings of the section `d` shows.
pub(crate) fn reset_section(d: &mut Dialog) {
    match d.str("section").as_str() {
        "basic" => reset_basic(d),
        "advanced" => reset_advanced(d),
        "color" => reset_color(d),
        "opentype" => reset_opentype(d),
        _ => {}
    }
}

pub fn reset_panel(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = app.ui.dialog.as_mut().ok_or("no dialog open")?;
    let character_new = d.kind == super::new_text_style::KIND && !d.bool("__para");
    if (d.kind != KIND && !character_new) || !can_reset(d) {
        return Err("the open dialog has no Reset".into());
    }
    reset_section(d);
    Ok(json!({"dialog": d.kind}))
}

/// The section list of Character Style Options and New Character Style.
pub(crate) fn section_list(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.vertical(|ui| {
        for (id, label) in [
            ("general", tl!("General")),
            ("basic", tl!("Basic Character Formats")),
            ("advanced", tl!("Advanced Character Formats")),
            ("color", tl!("Character Color")),
            ("opentype", tl!("OpenType Features")),
        ] {
            if ui.selectable_label(d.str("section") == id, label).clicked() {
                d.fields.insert("section".into(), json!(id));
            }
        }
    });
}

/// The Reset Panel button, enabled on every section but General.
pub(crate) fn reset_button(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.add_enabled_ui(can_reset(d), |ui| {
        if ui.button(tl!("Reset Panel")).clicked() {
            reset_section(d);
        }
    });
}

fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[(&str, &str)]) {
    let selected = d.fields.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let shown = options.iter().find(|(value, _)| *value == selected).map_or("", |(_, label)| *label);
    let labels: Vec<&str> = std::iter::once(" ").chain(options.iter().map(|(_, label)| *label)).collect();
    if let Some(i) = crate::widgets::dropdown(ui, (KIND, key), shown, &labels, 190.0) {
        d.fields.insert(key.into(), if i == 0 { Value::Null } else { options.get(i - 1).map_or(Value::Null, |(value, _)| json!(value)) });
    }
}

pub(crate) fn basic_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("character-style-basic").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        for (key, caption) in [
            ("font_family", tl!("Font Family:")),
            ("font_style", tl!("Font Style:")),
            ("size", tl!("Size")),
            ("leading", tl!("Leading")),
            ("kerning", tl!("Kerning")),
            ("tracking", tl!("Tracking")),
        ] {
            ui.label(caption);
            if d.fields.get(key).is_some_and(Value::is_null) {
                d.fields.insert(key.into(), json!(""));
            }
            form::text(ui, d, key, 190.0);
            ui.end_row();
        }
        ui.label(tl!("Case"));
        choice(ui, d, "case", &[("normal", tl!("Normal")), ("allCaps", tl!("All Caps")), ("smallCaps", tl!("Small Caps"))]);
        ui.end_row();
        ui.label(tl!("Position"));
        choice(ui, d, "position", &[("normal", tl!("Normal")), ("superscript", tl!("Superscript")), ("subscript", tl!("Subscript"))]);
        ui.end_row();
        for (key, caption) in [
            ("underline", tl!("Underline")),
            ("strikethrough", tl!("Strikethrough")),
            ("standardVerticalRoman", tl!("Standard Vertical Roman Alignment")),
        ] {
            ui.label("");
            let value = d.fields.get(key).and_then(Value::as_bool);
            if crate::widgets::check3(ui, caption, value, true) {
                d.fields.insert(
                    key.into(),
                    match value {
                        None => json!(true),
                        Some(true) => json!(false),
                        Some(false) => Value::Null,
                    },
                );
            }
            ui.end_row();
        }
        ui.label(tl!("Character Alignment"));
        choice(
            ui,
            d,
            "charAlign",
            &[
                ("romanBaseline", tl!("Roman Baseline")),
                ("emBoxTop", tl!("Em Box Top/Right")),
                ("emBoxCenter", tl!("Em Box Center")),
                ("emBoxBottom", tl!("Em Box Bottom/Left")),
                ("icfTop", tl!("ICF Top/Right")),
                ("icfBottom", tl!("ICF Bottom/Left")),
            ],
        );
        ui.end_row();
    });
}

/// Advanced Character Formats: Horizontal Scale, Vertical Scale, Baseline Shift, Character Rotation
/// and Language, each empty (unset) or set.
pub(crate) fn advanced_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("character-style-advanced").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        for (key, caption, unit) in [
            ("h_scale", tl!("Horizontal Scale"), "%"),
            ("v_scale", tl!("Vertical Scale"), "%"),
            ("baseline_shift", tl!("Baseline Shift"), "pt"),
            ("rotation", tl!("Character Rotation"), "°"),
        ] {
            ui.label(caption);
            if d.fields.get(key).is_some_and(Value::is_null) {
                d.fields.insert(key.into(), json!(""));
            }
            ui.horizontal(|ui| {
                form::text(ui, d, key, 160.0);
                ui.label(unit);
            });
            ui.end_row();
        }
        ui.label(tl!("Language:"));
        let names: Vec<&str> = std::iter::once(" ").chain(vectorcraft_doc::setup::LANGUAGES.iter().map(|(name, _, _)| *name)).collect();
        let current = d.fields.get("language").and_then(Value::as_str).unwrap_or("").to_string();
        if let Some(i) = crate::widgets::dropdown(ui, (KIND, "language"), &current, &names, 190.0) {
            d.fields.insert("language".into(), if i == 0 { Value::Null } else { names.get(i).map_or(Value::Null, |name| json!(name)) });
        }
        ui.end_row();
    });
}

/// An on, off or unset checkbox in its own grid row: a click on unset turns it on, on turns it
/// off, and off makes it unset.
fn switch(ui: &mut egui::Ui, d: &mut Dialog, key: &str, caption: &str, enabled: bool) {
    ui.label("");
    let value = d.fields.get(key).and_then(Value::as_bool);
    if crate::widgets::check3(ui, caption, value, enabled) {
        let next = match value {
            None => json!(true),
            Some(true) => json!(false),
            Some(false) => Value::Null,
        };
        d.fields.insert(key.into(), next);
    }
    ui.end_row();
}

/// The Fill or Stroke menu (`key`): unset, None, or a swatch of the document. A paint no swatch
/// gives (a gradient, a pattern, a color from a file) shows as its description and stays as it
/// is until another entry is chosen.
fn paint_choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str) {
    let swatches: Vec<(String, Value)> = d
        .fields
        .get("__swatches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|pair| Some((pair.get(0)?.as_str()?.to_string(), pair.get(1)?.clone())))
        .collect();
    let current = d.fields.get(key).cloned().unwrap_or(Value::Null);
    let none = tl!("None");
    let shown = if current.is_null() {
        String::new()
    } else if current.get("type").and_then(Value::as_str) == Some("none") {
        none.to_string()
    } else if let Some((name, _)) = swatches.iter().find(|(_, paint)| *paint == current) {
        name.clone()
    } else if let Some((name, _)) = swatches.iter().find(|(_, paint)| paint.get("swatch").is_some_and(|s| Some(s) == current.get("swatch"))) {
        name.clone()
    } else {
        serde_json::from_value::<vectorcraft_color::Paint>(current).map(|p| p.label()).unwrap_or_default()
    };
    let options: Vec<&str> = [" ", none].into_iter().chain(swatches.iter().map(|(name, _)| name.as_str())).collect();
    if let Some(i) = crate::widgets::dropdown_names(ui, (KIND, key), &shown, &options, 190.0) {
        let paint = match i {
            0 => Value::Null,
            1 => json!({"type": "none"}),
            _ => swatches.get(i - 2).map_or(Value::Null, |(_, paint)| paint.clone()),
        };
        d.fields.insert(format!("{key}_tint"), tint_field(&paint));
        d.fields.insert(key.into(), paint);
    }
}

/// Character Color: the Fill and Stroke paints, the Tint of a paint linked to a global swatch,
/// the stroke Weight and Overprint Fill and Overprint Stroke, each unset or set.
pub(crate) fn color_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("character-style-color").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        for (key, caption) in [("fill", tl!("Fill")), ("stroke", tl!("Stroke"))] {
            ui.label(caption);
            paint_choice(ui, d, key);
            ui.end_row();
            let tint_key = format!("{key}_tint");
            ui.label(tl!("Tint"));
            if d.fields.get(&tint_key).is_none_or(Value::is_null) {
                d.fields.insert(tint_key.clone(), json!(""));
            }
            let linked = d.fields.get(key).and_then(|p| p.get("swatch")).is_some_and(Value::is_string);
            ui.add_enabled_ui(linked, |ui| {
                ui.horizontal(|ui| {
                    form::text(ui, d, &tint_key, 160.0);
                    ui.label("%");
                });
            });
            ui.end_row();
        }
        ui.label(tl!("Weight"));
        if d.fields.get("stroke_width").is_none_or(Value::is_null) {
            d.fields.insert("stroke_width".into(), json!(""));
        }
        let stroked = d.fields.get("stroke").and_then(|p| p.get("type")).and_then(Value::as_str) != Some("none");
        ui.add_enabled_ui(stroked, |ui| {
            ui.horizontal(|ui| {
                form::text(ui, d, "stroke_width", 160.0);
                ui.label("pt");
            });
        });
        ui.end_row();
        switch(ui, d, "overprint_fill", tl!("Overprint Fill"), true);
        switch(ui, d, "overprint_stroke", tl!("Overprint Stroke"), true);
    });
}

/// OpenType Features: the Figure and Position menus, the feature checkboxes and the stylistic
/// sets, each unset or set.
pub(crate) fn opentype_body(ui: &mut egui::Ui, d: &mut Dialog) {
    let figures: Vec<(&str, &str)> = FIGURE_VALUES.into_iter().zip(crate::panels::opentype::FIGURES).collect();
    let positions: Vec<(&str, &str)> = POSITION_VALUES.into_iter().zip(crate::panels::opentype::POSITIONS).collect();
    egui::Grid::new("character-style-opentype").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Figure:"));
        choice(ui, d, "ot_figure", &figures);
        ui.end_row();
        ui.label(tl!("Position:"));
        choice(ui, d, "ot_position", &positions);
        ui.end_row();
        for (key, caption) in [
            ("ot_liga", tl!("Standard Ligatures")),
            ("ot_calt", tl!("Contextual Alternates")),
            ("ot_dlig", tl!("Discretionary Ligatures")),
            ("ot_swsh", tl!("Swash")),
            ("ot_salt", tl!("Stylistic Alternates")),
            ("ot_titl", tl!("Titling Alternates")),
            ("ot_ordn", tl!("Ordinals")),
            ("ot_frac", tl!("Fractions")),
        ] {
            switch(ui, d, key, caption, true);
        }
    });
    ui.add_space(6.0);
    ui.label(tl!("Stylistic Sets"));
    // Each set is on (`ssNN`), off (`-ssNN`) or unset (not listed), as the other checkboxes are.
    let mut listed: Vec<String> =
        d.fields.get("ot_sets").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    let mut clicked = None;
    egui::Grid::new("character-style-sets").num_columns(5).spacing([12.0, 4.0]).show(ui, |ui| {
        for (i, set) in vectorcraft_text::EXTRA_FEATURES[..20].iter().enumerate() {
            let value = listed.iter().find(|t| bare(t) == *set).map(|t| !t.starts_with('-'));
            if crate::widgets::check3(ui, set, value, true) {
                clicked = Some((*set, value));
            }
            if i % 5 == 4 {
                ui.end_row();
            }
        }
    });
    if let Some((set, value)) = clicked {
        listed.retain(|t| bare(t) != set);
        match value {
            None => listed.push(set.to_string()),
            Some(true) => listed.push(format!("-{set}")),
            Some(false) => {}
        }
        d.fields.insert("ot_sets".into(), if listed.is_empty() { Value::Null } else { json!(listed) });
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    ui.horizontal(|ui| {
        section_list(ui, d);
        ui.separator();
        ui.vertical(|ui| {
            if d.str("section") == "basic" {
                basic_body(ui, d);
            } else if d.str("section") == "advanced" {
                advanced_body(ui, d);
            } else if d.str("section") == "color" {
                color_body(ui, d);
            } else if d.str("section") == "opentype" {
                opentype_body(ui, d);
            } else {
                ui.horizontal(|ui| {
                    ui.label(tl!("Style Name:"));
                    form::text(ui, d, "name", 190.0);
                });
                ui.separator();
                if let Ok(attrs) = edited_attrs(d) {
                    for (key, value) in attrs {
                        ui.label(format!("{}: {}", super::humanize(&key), value));
                    }
                }
            }
        });
    });
    reset_button(ui, d);
    match edited_attrs(d) {
        Ok(attrs) => {
            let params = json!({"name": d.str("__style"), "to": d.str("name"), "attrs": attrs});
            form::preview(app, ui, d, tl!("Character Style Options"), "charStyle.edit", params);
        }
        Err(why) => attrs_error(ui, &why),
    }
    false
}

/// The message `edited_attrs` gave for a field value it does not accept, drawn under the dialog.
pub(crate) fn attrs_error(ui: &mut egui::Ui, why: &str) {
    ui.add_space(6.0);
    ui.colored_label(crate::theme::Tokens::get(ui.ctx()).error, crate::i18n::message(crate::i18n::current(), why));
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let attrs = edited_attrs(d)?;
    form::commit_preview(app, "charStyle.edit", json!({"name": d.str("__style"), "to": d.str("name"), "attrs": attrs}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn unchanged_basic_fields_preserve_original_keys() {
        for attrs in [
            json!({"all_caps": false}),
            json!({"all_caps": true}),
            json!({"small_caps": 70.0}),
            json!({"font_family": "Example", "font_style": "Regular", "font_version": "1", "size": 24,
                "leading": null, "kerning": null, "tracking": 3, "underline": null,
                "strikethrough": false, "position": {"kind": "normal", "size": 80}, "charAlign": null}),
        ] {
            let original = attrs.as_object().unwrap().clone();
            let mut fields = fields_from_attrs(&original);
            fields.insert("__attrs".into(), attrs);
            let d = Dialog { kind: KIND.into(), fields };
            assert_eq!(edited_attrs(&d).unwrap(), original);
        }
        let original = json!({"all_caps": false}).as_object().unwrap().clone();
        let mut fields = fields_from_attrs(&original);
        fields.insert("__attrs".into(), json!(original));
        fields.insert("case".into(), json!("allCaps"));
        let d = Dialog { kind: KIND.into(), fields };
        assert_eq!(edited_attrs(&d).unwrap(), json!({"all_caps": true, "small_caps": null}).as_object().unwrap().clone());
    }

    #[test]
    fn unchanged_options_keep_local_small_caps_and_history() {
        for preview in [false, true] {
            let mut app = app();
            app.run("charStyle.new", json!({"name": "Body", "attrs": {"all_caps": false}})).unwrap();
            let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
            app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
            app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 4, "smallCaps": true})).unwrap();
            let state = app.session.doc().unwrap();
            let before = state.doc.clone();
            let history = (state.history.undo.len(), state.history.redo.len());
            open(&mut app, "Body").unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(preview));
            if preview {
                frame(&mut app);
                assert_eq!(app.session.doc().unwrap().doc, before);
            }
            super::super::confirm(&mut app).unwrap();
            let state = app.session.doc().unwrap();
            assert_eq!(state.doc, before);
            assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
            assert_eq!(
                state.doc.node(vectorcraft_doc::NodeId(id)).and_then(|node| match &node.kind {
                    vectorcraft_doc::NodeKind::Text(text) => text.runs.first().map(|run| run.style.small_caps),
                    _ => None,
                }),
                Some(Some(state.doc.setup.small_caps_size))
            );
        }
    }

    #[test]
    fn options_preview_cancel_reset_and_commit_keep_sparse_definition() {
        let mut app = app();
        app.run("charStyle.new", json!({"name":"Body","attrs":{"size":24.0,"baseline_shift":3.0}})).unwrap();
        let id = app.run("text.create", json!({"x":10,"y":40,"text":"Text"})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name":"Body","id":id})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        open(&mut app, "Body").unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("preview"));
        assert!(app.run("ui.charStyleOptions.resetPanel", json!({})).is_err());
        assert!(app.ui.dialog.as_ref().unwrap().fields["underline"].is_null());
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("kerning"), "");
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("name".into(), json!("Revised"));
            d.fields.insert("size".into(), json!(18));
        }
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].name, "Revised");
        super::super::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        open(&mut app, "Body").unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("section".into(), json!("basic"));
        }
        app.run("ui.charStyleOptions.resetPanel", json!({})).unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            assert_eq!(edited_attrs(d).unwrap(), serde_json::from_value(json!({"baseline_shift":3.0})).unwrap());
            d.fields.insert("name".into(), json!("Revised"));
        }
        super::super::confirm(&mut app).unwrap();
        let state = app.session.doc().unwrap();
        assert_eq!(state.history.undo.len(), undo + 1);
        assert_eq!(state.doc.char_styles[0].name, "Revised");
        assert_eq!(state.doc.char_styles[0].attrs, serde_json::from_value(json!({"baseline_shift":3.0})).unwrap());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].name, "Revised");
        app.run("select.set", json!({"ids":[id]})).unwrap();
        app.run("ui.newCharStyle", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("size"), "12.0");
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("size".into(), json!(36.0));
            d.fields.insert("name".into(), json!("Future"));
        }
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.char_styles.iter().find(|def| def.name == "Future").unwrap().attrs["size"], json!(36.0));
    }

    /// Basic Character Formats shows Character Alignment under its own name, and Standard Vertical
    /// Roman Alignment as its own on, off or unset checkbox that the style saves.
    #[test]
    fn standard_vertical_roman_alignment_has_its_own_row() {
        let mut d = Dialog::new(KIND, json!({}));
        d.fields.extend(fields_from_attrs(&Map::new()));
        d.fields.insert("__attrs".into(), json!({}));
        let mut app = app();
        let shown = crate::tests_labels::painted_text(&mut app, |_, ui| basic_body(ui, &mut d));
        assert!(shown.contains("Character Alignment"), "{shown}");
        assert!(shown.contains("Standard Vertical Roman Alignment"), "{shown}");
        assert!(d.fields["standardVerticalRoman"].is_null());
        assert_eq!(edited_attrs(&d).unwrap(), Map::new());
        d.fields.insert("standardVerticalRoman".into(), json!(true));
        d.fields.insert("charAlign".into(), json!("emBoxCenter"));
        assert_eq!(edited_attrs(&d).unwrap(), json!({"standardVerticalRoman": true, "charAlign": "emBoxCenter"}).as_object().unwrap().clone());
        let fields = fields_from_attrs(json!({"standardVerticalRoman": false}).as_object().unwrap());
        assert_eq!(fields["standardVerticalRoman"], json!(false));
        reset_basic(&mut d);
        assert!(d.fields["standardVerticalRoman"].is_null());

        app.run("charStyle.new", json!({"name": "Upright", "attrs": {}})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "AB", "vertical": true})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name": "Upright", "id": id})).unwrap();
        open(&mut app, "Upright").unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("standardVerticalRoman".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.char_styles[0].attrs["standardVerticalRoman"], json!(true));
        let vectorcraft_doc::NodeKind::Text(t) = &doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        assert!(t.runs.iter().all(|r| r.style.standard_vertical_roman));
    }

    fn attrs(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    fn dialog_for(original: Value) -> Dialog {
        let mut d = Dialog::new(KIND, json!({}));
        d.fields.extend(fields_from_attrs(&attrs(original.clone())));
        d.fields.insert("__attrs".into(), original);
        d
    }

    #[test]
    fn advanced_fields_show_set_values_and_keep_untouched_ones() {
        let d = dialog_for(json!({}));
        for key in ADVANCED_NUMBERS {
            assert_eq!(d.fields[*key], json!(""), "{key}");
        }
        assert!(d.fields["language"].is_null());
        assert_eq!(edited_attrs(&d).unwrap(), Map::new());
        for original in [
            json!({"h_scale": 100.0, "rotation": 0.0, "language": null}),
            json!({"v_scale": 50.0, "baseline_shift": -3.0, "language": "de"}),
            json!({"language": "x-file", "size": 12.0}),
            // Values a style captures from text in a scaled type object, Size and Leading below 0.1 included.
            json!({"h_scale": 20000.0, "baseline_shift": 3000.0, "size": 0.05, "leading": 0.05}),
        ] {
            let d = dialog_for(original.clone());
            assert_eq!(edited_attrs(&d).unwrap(), attrs(original.clone()), "{original}");
        }
        let d = dialog_for(json!({"h_scale": 100.0, "rotation": 0.0, "language": null}));
        assert_eq!((d.fields["h_scale"].clone(), d.fields["v_scale"].clone()), (json!(100.0), json!("")));
        assert_eq!(d.fields["language"], json!("English: USA"));
        assert_eq!(dialog_for(json!({"language": "de"})).fields["language"], json!("German"));
        assert_eq!(dialog_for(json!({"language": "x-file"})).fields["language"], json!("x-file"));

        let mut d = dialog_for(json!({"h_scale": 100.0, "language": "de"}));
        d.fields.insert("h_scale".into(), json!(""));
        d.fields.insert("v_scale".into(), json!("150"));
        d.fields.insert("baseline_shift".into(), json!("-4"));
        d.fields.insert("rotation".into(), json!("190"));
        d.fields.insert("language".into(), json!("English: USA"));
        assert_eq!(edited_attrs(&d).unwrap(), attrs(json!({"v_scale": 150.0, "baseline_shift": -4.0, "rotation": -170.0, "language": null})));
        d.fields.insert("language".into(), json!("French"));
        assert_eq!(edited_attrs(&d).unwrap()["language"], json!("fr"));
        d.fields.insert("language".into(), Value::Null);
        assert!(!edited_attrs(&d).unwrap().contains_key("language"));
    }

    #[test]
    fn character_style_numbers_outside_their_limits_are_errors() {
        for (key, value, message) in [
            ("size", "1296.1", "Size must be a number at most 1296"),
            ("leading", "5000.1", "Leading must be a number at most 5000"),
            ("h_scale", "0", "Horizontal Scale must be a number from 1 to 10000"),
            ("h_scale", "0.9", "Horizontal Scale must be a number from 1 to 10000"),
            ("v_scale", "10000.1", "Vertical Scale must be a number from 1 to 10000"),
            ("baseline_shift", "1296.1", "Baseline Shift must be a number from -1296 to 1296"),
            ("baseline_shift", "-1296.1", "Baseline Shift must be a number from -1296 to 1296"),
            ("rotation", "inf", "Character Rotation must be a number"),
            ("h_scale", "wide", "Horizontal Scale must be a number"),
        ] {
            let mut d = dialog_for(json!({}));
            d.fields.insert(key.into(), json!(value));
            assert_eq!(edited_attrs(&d).unwrap_err(), message, "{key} {value}");
        }
        let mut d = dialog_for(json!({}));
        for (key, value) in
            [("size", "1296"), ("leading", "5000"), ("h_scale", "1"), ("v_scale", "10000"), ("baseline_shift", "-1296"), ("rotation", "-200")]
        {
            d.fields.insert(key.into(), json!(value));
        }
        assert_eq!(
            edited_attrs(&d).unwrap(),
            attrs(json!({"size": 1296.0, "leading": 5000.0, "h_scale": 1.0, "v_scale": 10000.0, "baseline_shift": -1296.0, "rotation": 160.0}))
        );
    }

    #[test]
    fn advanced_section_shows_its_five_controls() {
        let mut d = dialog_for(json!({}));
        let mut app = app();
        let shown = crate::tests_labels::painted_text(&mut app, |_, ui| advanced_body(ui, &mut d));
        for label in ["Horizontal Scale", "Vertical Scale", "Baseline Shift", "Character Rotation", "Language:"] {
            assert!(shown.contains(label), "{label}: {shown}");
        }
    }

    #[test]
    fn advanced_options_preview_cancel_reset_and_commit() {
        let mut app = app();
        app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0, "h_scale": 90.0, "baseline_shift": 0.0}})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
        app.run("text.setRangeStyle", json!({"id": id, "start": 2, "end": 4, "baselineShift": 5.0})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        open(&mut app, "Body").unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["h_scale"], json!(90.0));
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("rotation"), "");
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("section".into(), json!("advanced"));
            d.fields.insert("h_scale".into(), json!("80"));
            d.fields.insert("baseline_shift".into(), json!("2"));
            d.fields.insert("rotation".into(), json!("45"));
            d.fields.insert("language".into(), json!("Turkish"));
        }
        assert!(crate::menus::enabled(&app, "ui.charStyleOptions.resetPanel"));
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["rotation"], json!(45.0));
        super::super::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

        open(&mut app, "Body").unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("section".into(), json!("advanced"));
            d.fields.insert("size".into(), json!(18));
            d.fields.insert("v_scale".into(), json!("70"));
        }
        app.run("ui.charStyleOptions.resetPanel", json!({})).unwrap();
        assert_eq!(edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap(), attrs(json!({"size": 18.0})));
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("h_scale".into(), json!("80"));
            d.fields.insert("v_scale".into(), json!("60"));
            d.fields.insert("baseline_shift".into(), json!("2"));
            d.fields.insert("rotation".into(), json!("45"));
            d.fields.insert("language".into(), json!("Turkish"));
        }
        super::super::confirm(&mut app).unwrap();
        let state = app.session.doc().unwrap();
        assert_eq!(state.history.undo.len(), undo + 1);
        assert_eq!(
            state.doc.char_styles[0].attrs,
            attrs(json!({"size": 18.0, "h_scale": 80.0, "v_scale": 60.0, "baseline_shift": 2.0, "rotation": 45.0, "language": "tr"}))
        );
        let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        assert_eq!(
            t.runs
                .iter()
                .map(|r| (r.style.h_scale, r.style.v_scale, r.style.baseline_shift, r.style.rotation, r.style.language.clone()))
                .collect::<Vec<_>>(),
            [(80.0, 60.0, 2.0, 45.0, Some("tr".to_string())), (80.0, 60.0, 5.0, 45.0, Some("tr".to_string()))]
        );
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
    }

    #[test]
    fn new_character_style_sets_and_resets_advanced_formats() {
        let mut app = app();
        app.run("ui.newCharStyle", json!({})).unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("section".into(), json!("advanced"));
            d.fields.insert("v_scale".into(), json!("130"));
        }
        app.run("ui.charStyleOptions.resetPanel", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("v_scale"), "");
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("name".into(), json!("Raised"));
            d.fields.insert("baseline_shift".into(), json!("3"));
            d.fields.insert("language".into(), json!("French"));
        }
        super::super::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let def = doc.char_styles.iter().find(|def| def.name == "Raised").unwrap();
        assert_eq!((def.attrs["baseline_shift"].clone(), def.attrs["language"].clone()), (json!(3.0), json!("fr")));
        assert!(!def.attrs.contains_key("v_scale"));
    }

    /// The negated tags of the twenty stylistic sets: an explicit empty Stylistic Sets choice.
    fn no_sets() -> Vec<String> {
        vectorcraft_text::EXTRA_FEATURES[..20].iter().map(|s| format!("-{s}")).collect()
    }

    #[test]
    fn color_and_opentype_fields_keep_unset_explicit_and_untouched_values() {
        let d = dialog_for(json!({}));
        for key in ["fill", "stroke", "overprint_fill", "overprint_stroke", "ot_figure", "ot_position", "ot_liga", "ot_frac", "ot_sets"] {
            assert!(d.fields[key].is_null(), "{key}");
        }
        assert_eq!((d.str("stroke_width"), d.str("fill_tint")), (String::new(), String::new()));
        assert_eq!(edited_attrs(&d).unwrap(), Map::new());

        let mut explicit =
            vec!["-tnum", "-onum", "-pnum", "-lnum", "-sups", "-subs", "-numr", "-dnom", "-liga"].into_iter().map(String::from).collect::<Vec<_>>();
        explicit.extend(no_sets());
        for original in [
            json!({"fill": {"type": "none"}, "stroke_width": 0.0, "overprint_fill": false, "features": explicit}),
            json!({"stroke": {"type": "pattern", "pattern": "Dots"}, "fill": {"type": "solid", "color": {"model": "gray", "k": 0.5}}}),
            json!({"features": ["zzzz", "dlig", "ss03"], "size": 9.0}),
        ] {
            let d = dialog_for(original.clone());
            assert_eq!(edited_attrs(&d).unwrap(), attrs(original.clone()), "{original}");
        }
        let d = dialog_for(json!({"fill": {"type": "none"}, "overprint_fill": false, "features": explicit}));
        assert_eq!(d.fields["fill"], json!({"type": "none"}));
        assert_eq!(d.fields["overprint_fill"], json!(false));
        assert_eq!((d.str("ot_figure"), d.str("ot_position")), ("default".to_string(), "default".to_string()));
        assert_eq!(
            (d.fields["ot_liga"].clone(), d.fields["ot_dlig"].clone(), d.fields["ot_sets"].clone()),
            (json!(false), Value::Null, json!(no_sets()))
        );

        // One changed control keeps every other tag, unknown ones included.
        let mut d = dialog_for(json!({"features": ["zzzz", "dlig", "ss03"], "stroke": {"type": "pattern", "pattern": "Dots"}}));
        assert_eq!((d.fields["ot_dlig"].clone(), d.fields["ot_sets"].clone()), (json!(true), json!(["ss03"])));
        d.fields.insert("ot_liga".into(), json!(false));
        d.fields.insert("overprint_stroke".into(), json!(true));
        assert_eq!(
            edited_attrs(&d).unwrap(),
            attrs(json!({"features": ["zzzz", "dlig", "ss03", "-liga"], "stroke": {"type": "pattern", "pattern": "Dots"}, "overprint_stroke": true}))
        );
        d.fields.insert("ot_sets".into(), json!(["ss05", "ss03"]));
        d.fields.insert("ot_figure".into(), json!("proportionalOldstyle"));
        d.fields.insert("ot_position".into(), json!("numerator"));
        d.fields.insert("ot_dlig".into(), Value::Null);
        let features = edited_attrs(&d).unwrap()["features"].clone();
        // Sets the list leaves out stay unset, so text that uses the style keeps its own.
        assert_eq!(features, json!(["zzzz", "-liga", "onum", "pnum", "-tnum", "-lnum", "-sups", "-subs", "numr", "-dnom", "ss05", "ss03"]));
        let reopened = dialog_for(json!({"features": features}));
        assert_eq!(
            ["ot_figure", "ot_position", "ot_liga", "ot_dlig"].map(|k| reopened.fields[k].clone()),
            [json!("proportionalOldstyle"), json!("numerator"), json!(false), Value::Null]
        );
        assert_eq!(reopened.fields["ot_sets"], json!(["ss03", "ss05"]));
        // Every control unset leaves the attribute unset.
        let mut d = dialog_for(json!({"features": ["dlig"]}));
        d.fields.insert("ot_dlig".into(), Value::Null);
        assert_eq!(edited_attrs(&d).unwrap(), Map::new());
    }

    #[test]
    fn figure_tags_no_menu_entry_matches_leave_the_figure_unset() {
        for features in [json!(["pnum"]), json!(["lnum"]), json!(["onum", "lnum"]), json!(["tnum", "pnum"])] {
            let mut d = dialog_for(json!({ "features": features }));
            assert!(d.fields["ot_figure"].is_null(), "{features}");
            assert_eq!(edited_attrs(&d).unwrap(), attrs(json!({ "features": features })), "{features}");
            d.fields.insert("ot_figure".into(), json!("default"));
            assert_eq!(edited_attrs(&d).unwrap()["features"], json!(["-tnum", "-onum", "-pnum", "-lnum"]), "{features}");
        }
        for (features, shown) in [(json!(["tnum"]), "tabularLining"), (json!(["-pnum"]), "default"), (json!(["pnum", "lnum"]), "proportionalLining")]
        {
            assert_eq!(dialog_for(json!({ "features": features })).str("ot_figure"), shown, "{features}");
        }
    }

    #[test]
    fn color_tint_and_weight_outside_their_limits_are_errors() {
        let linked = json!({"type": "solid", "color": {"model": "gray", "k": 0.5}, "swatch": "Ink"});
        for (key, value, message) in [
            ("fill_tint", "150", "Tint must be a number from 0 to 100"),
            ("stroke_tint", "-1", "Tint must be a number from 0 to 100"),
            ("stroke_width", "-1", "Weight must be a number from 0 to 4000000"),
            ("stroke_width", "thick", "Weight must be a number"),
        ] {
            let mut d = dialog_for(json!({"fill": linked, "stroke": linked}));
            d.fields.insert(key.into(), json!(value));
            assert_eq!(edited_attrs(&d).unwrap_err(), message, "{key} {value}");
        }
        let mut d = dialog_for(json!({"fill": linked}));
        assert_eq!(d.fields["fill_tint"], json!(100.0));
        d.fields.insert("fill_tint".into(), json!("40"));
        d.fields.insert("stroke_width".into(), json!("1.5"));
        assert_eq!(
            edited_attrs(&d).unwrap(),
            attrs(json!({"fill": {"type": "solid", "color": {"model": "gray", "k": 0.5}, "swatch": "Ink", "tint": 0.4}, "stroke_width": 1.5}))
        );
    }

    #[test]
    fn reset_panel_in_color_and_opentype_clears_only_that_section() {
        let original = json!({"size": 12.0, "h_scale": 90.0, "fill": {"type": "none"}, "stroke_width": 2.0, "overprint_stroke": true, "features": ["dlig", "zzzz"]});
        let mut d = dialog_for(original.clone());
        d.fields.insert("section".into(), json!("color"));
        assert!(can_reset(&d));
        reset_section(&mut d);
        assert_eq!(edited_attrs(&d).unwrap(), attrs(json!({"size": 12.0, "h_scale": 90.0, "features": ["dlig", "zzzz"]})));
        let mut d = dialog_for(original);
        d.fields.insert("section".into(), json!("opentype"));
        reset_section(&mut d);
        assert!(d.fields["ot_dlig"].is_null());
        assert_eq!(
            edited_attrs(&d).unwrap(),
            attrs(json!({"size": 12.0, "h_scale": 90.0, "fill": {"type": "none"}, "stroke_width": 2.0, "overprint_stroke": true}))
        );
        d.fields.insert("ot_frac".into(), json!(true));
        assert_eq!(edited_attrs(&d).unwrap()["features"], json!(["frac"]));
    }

    #[test]
    fn color_and_opentype_sections_show_their_controls() {
        let mut d = dialog_for(json!({}));
        let mut app = app();
        let shown = crate::tests_labels::painted_text(&mut app, |_, ui| section_list(ui, &mut d));
        for label in ["Character Color", "OpenType Features"] {
            assert!(shown.contains(label), "{label}: {shown}");
        }
        let shown = crate::tests_labels::painted_text(&mut app, |_, ui| color_body(ui, &mut d));
        for label in ["Fill", "Stroke", "Tint", "Weight", "Overprint Fill", "Overprint Stroke"] {
            assert!(shown.contains(label), "{label}: {shown}");
        }
        let shown = crate::tests_labels::painted_text(&mut app, |_, ui| opentype_body(ui, &mut d));
        for label in [
            "Figure:",
            "Position:",
            "Standard Ligatures",
            "Contextual Alternates",
            "Discretionary Ligatures",
            "Swash",
            "Stylistic Alternates",
            "Titling Alternates",
            "Ordinals",
            "Fractions",
            "Stylistic Sets",
            "ss01",
            "ss20",
        ] {
            assert!(shown.contains(label), "{label}: {shown}");
        }
        assert!(!shown.contains("Small Caps"), "{shown}");
    }

    #[test]
    fn color_and_features_preview_cancel_commit_and_new_style() {
        let mut app = app();
        app.run("swatch.new", json!({"name": "Brand", "color": "#2050a0", "global": true})).unwrap();
        app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let brand = json!(before.tint_paint("Brand", 1.0).unwrap());
        open(&mut app, "Body").unwrap();
        let choices = app.ui.dialog.as_ref().unwrap().fields["__swatches"].as_array().unwrap().clone();
        assert!(choices.contains(&json!(["Brand", brand])));
        let registration = vectorcraft_color::swatch::REGISTRATION;
        assert_eq!(choices.first(), Some(&json!([registration, before.tint_paint(registration, 1.0).unwrap()])));
        let set = |app: &mut VectorcraftApp| {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("section".into(), json!("color"));
            d.fields.insert("fill".into(), brand.clone());
            d.fields.insert("stroke".into(), brand.clone());
            d.fields.insert("stroke_tint".into(), json!("40"));
            d.fields.insert("stroke_width".into(), json!("2"));
            d.fields.insert("overprint_fill".into(), json!(true));
            d.fields.insert("ot_dlig".into(), json!(true));
            d.fields.insert("ot_figure".into(), json!("tabularLining"));
        };
        set(&mut app);
        assert!(crate::menus::enabled(&app, "ui.charStyleOptions.resetPanel"));
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["fill"], brand);
        super::super::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);

        open(&mut app, "Body").unwrap();
        set(&mut app);
        super::super::confirm(&mut app).unwrap();
        let state = app.session.doc().unwrap();
        assert_eq!(state.history.undo.len(), undo + 1);
        let tint40 = state.doc.tint_paint("Brand", 0.4).unwrap();
        let def = state.doc.char_styles[0].attrs.clone();
        assert_eq!(def["stroke"], json!(tint40));
        assert_eq!(def["features"], json!(["dlig", "tnum", "lnum", "-onum", "-pnum"]));
        let vectorcraft_doc::NodeKind::Text(t) = &state.doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        for run in &t.runs {
            assert_eq!(
                (run.style.fill.clone(), run.style.stroke.clone(), run.style.stroke_width),
                (state.doc.tint_paint("Brand", 1.0).unwrap(), tint40.clone(), 2.0)
            );
            assert_eq!(run.style.features, ["dlig", "tnum", "lnum", "-onum", "-pnum"]);
        }
        open(&mut app, "Body").unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.fields["stroke_tint"].clone(), d.str("ot_figure")), (json!(40.0), "tabularLining".to_string()));
        super::super::cancel(&mut app);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);

        app.run("select.set", json!({"ids": []})).unwrap();
        app.run("ui.newCharStyle", json!({})).unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            assert!(d.fields["fill"].is_null() && d.fields["ot_sets"].is_null());
            d.fields.insert("name".into(), json!("Ink"));
            d.fields.insert("section".into(), json!("opentype"));
            d.fields.insert("ot_sets".into(), json!(["-ss02", "ss04"]));
            d.fields.insert("ot_frac".into(), json!(false));
        }
        assert!(crate::menus::enabled(&app, "ui.charStyleOptions.resetPanel"));
        super::super::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        let def = doc.char_styles.iter().find(|def| def.name == "Ink").unwrap();
        assert_eq!(def.attrs["features"], json!(["-frac", "-ss02", "ss04"]));
        let vectorcraft_doc::NodeKind::Text(t) = &doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        assert!(t.runs.iter().all(|r| r.style.style_name.as_deref() == Some("Body")));
    }

    #[test]
    fn invalid_numbers_name_the_visible_field() {
        let mut d = Dialog::new(KIND, json!({}));
        for (key, label) in [("size", "Size"), ("leading", "Leading"), ("kerning", "Kerning"), ("tracking", "Tracking")] {
            d.fields.insert(key.into(), json!("not a number"));
            assert_eq!(optional_number(&d, key).unwrap_err(), format!("{label} must be a number"));
        }
    }

    #[test]
    fn size_and_leading_below_minimum_do_not_preview_or_commit() {
        for (key, value) in [("size", json!(0)), ("size", json!(-2)), ("leading", json!(0.09)), ("leading", json!(-1))] {
            let mut app = app();
            app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0, "leading": 28.0}})).unwrap();
            let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
            app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
            let before = app.session.doc().unwrap().doc.clone();
            open(&mut app, "Body").unwrap();
            {
                let d = app.ui.dialog.as_mut().unwrap();
                d.fields.insert("name".into(), json!("Revised"));
                d.fields.insert(key.into(), value);
                assert!(edited_attrs(d).unwrap_err().contains("at least 0.1"));
            }
            frame(&mut app);
            assert_eq!(app.session.doc().unwrap().doc, before, "{key} preview");
            assert!(super::super::confirm(&mut app).is_err(), "{key} confirm");
            assert_eq!(app.session.doc().unwrap().doc, before, "{key} confirm");
        }
    }

    #[test]
    fn new_character_style_shows_size_and_leading_errors_before_ok() {
        use egui_kittest::kittest::Queryable;
        for (key, value) in [("size", json!(0)), ("leading", json!(0.05))] {
            let mut app = app();
            app.run("ui.newCharStyle", json!({})).unwrap();
            let why = {
                let d = app.ui.dialog.as_mut().unwrap();
                d.fields.insert("section".into(), json!("basic"));
                d.fields.insert(key.into(), value);
                edited_attrs(d).unwrap_err()
            };
            let shown = crate::i18n::message(crate::i18n::current(), &why);
            let mut fonts = false;
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_accessibility_check(false).build_ui_state(
                move |ui, app: &mut VectorcraftApp| {
                    if !std::mem::replace(&mut fonts, true) {
                        crate::theme::install_fonts(ui.ctx());
                        return;
                    }
                    super::super::show(app, ui.ctx());
                },
                app,
            );
            h.run_steps(4);
            assert!(h.query_by_label(&shown).is_some(), "{key}: {shown} is not shown");
            let app = h.state_mut();
            let styles = app.session.doc().unwrap().doc.char_styles.len();
            assert!(super::super::confirm(app).is_err(), "{key} confirm");
            assert!(app.ui.dialog.is_some(), "{key} dialog stays open");
            assert_eq!(app.session.doc().unwrap().doc.char_styles.len(), styles, "{key} style count");
        }
    }

    #[test]
    fn invalid_number_keeps_last_valid_preview() {
        let mut app = app();
        app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
        open(&mut app, "Body").unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("name".into(), json!("Revised"));
            d.fields.insert("size".into(), json!(18));
        }
        frame(&mut app);
        let valid = app.session.doc().unwrap().doc.clone();
        assert_eq!(valid.char_styles[0].name, "Revised");
        assert_eq!(valid.char_styles[0].attrs["size"], json!(18.0));
        app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!("invalid"));
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, valid);
    }

    #[test]
    fn script_positions_use_document_setup_in_existing_and_new_style_dialogs() {
        let mut app = app();
        app.run(
            "document.setup",
            json!({"superscript": {"size": 67.0, "position": 22.0},
            "subscript": {"size": 72.0, "position": 18.0}}),
        )
        .unwrap();
        app.run("charStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
        open(&mut app, "Body").unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("position".into(), json!("superscript"));
        assert_eq!(
            edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap()["position"],
            json!({"kind": "superscript", "size": 67.0, "position": 22.0})
        );
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["position"], json!({"kind": "superscript", "size": 67.0, "position": 22.0}));
        app.run("ui.newCharStyle", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("position".into(), json!("subscript"));
        assert_eq!(edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap()["position"], json!({"kind": "subscript", "size": 72.0, "position": 18.0}));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(
            app.session.doc().unwrap().doc.char_styles.last().unwrap().attrs["position"],
            json!({"kind": "subscript", "size": 72.0, "position": 18.0})
        );
    }
}
