//! Character Style Options fields shared by editing and creating a character style, as the tabs of a
//! hosted properties dialog, and the helpers the paragraph and New Style dialogs build theirs from.
//!
//! Each field is unset (empty, absent, or a checkbox in its unset state) or set, and an unset field leaves
//! its attribute out of the style. A field still as it opened keeps the style's stored value as it is.

use std::sync::Arc;

use craft::behavior::Mixed;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Outcome as _, Props as _, Show as _};
use craft::ui::dialog::{DialogResult, FieldRefusal};
use craft::ui::patterns::form_dialogs::{
    DialogFields, FieldValue, HostedPropertiesDialog, Param, ParamKind, PropertiesDialog, PropertiesTab, PropertyRow, TabLayout,
};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value, json};
use vectorcraft_doc::setup::DocSetup;

use super::preview::LivePreview;
use crate::VectorcraftApp;

pub const KIND: &str = "charStyleOptions";
/// The section ids, in the order the tabs show them.
pub(crate) const SECTIONS: [&str; 5] = ["general", "basic", "advanced", "color", "opentype"];
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
const ADVANCED: &[&str] = &[
    "h_scale",
    "v_scale",
    "baseline_shift",
    "tcyUpDown",
    "tcyLeftRight",
    "rotation",
    "akiLeft",
    "akiRight",
    "tsume",
    "diacriticHorizontal",
    "diacriticVertical",
    "language",
];
/// The number fields of Advanced Character Formats.
const ADVANCED_NUMBERS: &[&str] = &[
    "h_scale",
    "v_scale",
    "baseline_shift",
    "tcyUpDown",
    "tcyLeftRight",
    "rotation",
    "akiLeft",
    "akiRight",
    "tsume",
    "diacriticHorizontal",
    "diacriticVertical",
];
/// The Insert Aki Left and Right fields, which stop at [`vectorcraft_doc::text::MAX_AKI`].
const AKI: [&str; 2] = ["akiLeft", "akiRight"];
/// The Adjust Horizontal and Vertical Diacritic Position fields, which stop at
/// [`vectorcraft_doc::text::MAX_DIACRITIC_POSITION`] either way.
const DIACRITIC: [&str; 2] = ["diacriticHorizontal", "diacriticVertical"];
/// The attributes of Character Color.
const COLOR: &[&str] = &["fill", "stroke", "stroke_width", "overprint_fill", "overprint_stroke"];
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
    ("ot_hkna", "hkna"),
    ("ot_ital", "ital"),
];
/// The feature tags the Figure menu sets.
const FIGURE_TAGS: [&str; 4] = ["tnum", "onum", "pnum", "lnum"];
/// The values of the Figure field, in the order of the OpenType panel's Figure menu.
const FIGURE_VALUES: [&str; 5] = ["default", "tabularLining", "proportionalLining", "proportionalOldstyle", "tabularOldstyle"];
/// The values of the Position field, in the order of the OpenType panel's Position menu.
const POSITION_VALUES: [&str; 5] = ["default", "superscript", "subscript", "numerator", "denominator"];
/// The fields of OpenType Features.
const OT_FIELDS: &[&str] = &[
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
    "ot_hkna",
    "ot_ital",
    "ot_sets",
];

/// What a character style dialog opened with.
pub(crate) struct Opening {
    /// The style's name when the dialog opened; empty for a new style.
    pub(crate) style: String,
    /// The style's attributes when the dialog opened.
    pub(crate) attrs: Map<String, Value>,
    /// The document's Small Caps size (%), which a style that turns small caps on takes.
    small_caps_size: f64,
    /// The document's superscript size and position (%).
    superscript: [f64; 2],
    /// The document's subscript size and position (%).
    subscript: [f64; 2],
    /// The swatches Character Color offers ([`swatch_choices`]).
    swatches: Value,
}

impl Opening {
    /// The opening of style `style` with `attrs` in a document with `setup` and `swatches`.
    pub(crate) fn new(style: &str, attrs: Map<String, Value>, setup: &DocSetup, swatches: Value) -> Opening {
        Opening {
            style: style.into(),
            attrs,
            small_caps_size: setup.small_caps_size,
            superscript: [setup.superscript.size, setup.superscript.position],
            subscript: [setup.subscript.size, setup.subscript.position],
            swatches,
        }
    }

    /// The dialog's values at opening, for a style called `name`.
    pub(crate) fn fields(&self, name: &str) -> DialogFields {
        let mut fields = to_fields(&fields_from_attrs(&self.attrs));
        fields.set("name", FieldValue::Text(name.into()));
        paint_flags(&mut fields);
        fields
    }
}

/// The dialog's values as JSON, the form [`edited_attrs`] reads them in: an unset field (absent, `null` or
/// empty text) is absent, and a number is its text, as a text row holds it.
pub(crate) struct Values<'a> {
    map: Map<String, Value>,
    /// The values as the dialog keeps them, for the keys that are not style fields.
    pub(crate) dialog: &'a DialogFields,
}

impl<'a> Values<'a> {
    /// The values of `dialog`.
    pub(crate) fn new(dialog: &'a DialogFields) -> Self {
        let map = dialog.iter().filter_map(|(key, value)| Some((key.to_string(), set_value(value_of(value))?))).collect();
        Values { map, dialog }
    }

    /// The value of field `key`; `None` while it is unset.
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key)
    }

    /// The text of field `key`; empty while it is unset or not text.
    fn str(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or("")
    }

    /// Whether Reset Panel cleared tab `tab`: its unset fields are written as unset.
    pub(crate) fn reset(&self, tab: &str) -> bool {
        self.dialog.was_reset(tab)
    }
}

/// `value` unless it is unset: `null` or empty text.
fn set_value(value: Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        other => Some(other),
    }
}

/// A field's value as JSON.
pub(crate) fn value_of(value: &FieldValue) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The field value of a JSON `value` (`None` for `null`): a number is its JSON text (`12.0`), which the text rows hold.
fn field_of(value: &Value) -> Option<FieldValue> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => FieldValue::Bool(*b),
        Value::String(s) => FieldValue::Text(s.clone()),
        Value::Number(n) => FieldValue::Text(n.to_string()),
        other => FieldValue::Json(other.clone()),
    })
}

/// The dialog's values for the field map `fields`.
pub(crate) fn to_fields(fields: &Map<String, Value>) -> DialogFields {
    let mut out = DialogFields::new();
    for (key, value) in fields {
        if let Some(value) = field_of(value) {
            out.set(key.clone(), value);
        }
    }
    out
}

/// `fields` as [`Values`] reads the dialog's values, so the two compare equal for a field left as it opened.
pub(crate) fn canonical_map(fields: Map<String, Value>) -> Map<String, Value> {
    to_fields(&fields).iter().filter_map(|(key, value)| Some((key.to_string(), set_value(value_of(value))?))).collect()
}

/// A text row.
pub(crate) fn text(key: impl Into<String>, label: impl Into<Label>) -> Param {
    Param::new(key, label, ParamKind::text(""))
}

/// A menu of `options` (value, label) with an empty first entry for unset.
pub(crate) fn select(key: &'static str, label: &'static str, options: &[(&'static str, &'static str)]) -> Param {
    let unset = (String::new(), Label::from(" "));
    let entries = std::iter::once(unset).chain(options.iter().map(|(value, label)| ((*value).to_string(), Label::from(*label))));
    Param::new(key, label, ParamKind::choice(entries.collect(), None))
}

/// The rows of `params`.
pub(crate) fn rows(params: Vec<Param>) -> Vec<PropertyRow> {
    params.into_iter().map(PropertyRow::field).collect()
}

/// A check box that is on, off or unset (shown mixed): a click turns unset on, on off, and off unset.
fn tri_box(ui: &mut egui::Ui, current: Option<bool>, label: Option<Label>, salt: &str) -> (egui::Response, Option<Option<bool>>) {
    let shown = current.map_or(Mixed::Mixed, Mixed::Same);
    let mut check = Checkbox::new(Bind::given(shown)).id_salt(salt.to_string());
    if let Some(label) = label {
        check = check.label(label);
    }
    let out = check.show(ui);
    let next = out.value().is_some().then_some(match current {
        None => Some(true),
        Some(true) => Some(false),
        Some(false) => None,
    });
    (out.response().clone(), next)
}

/// The on, off or unset check box `key`; unset is an absent key.
// PLAN: `craft::behavior::Mixed` needs the `craft` facade in the app's Cargo.toml (no row gives it); `Mixed` is not re-exported by craft_ui.
pub(crate) fn tri_state(key: impl Into<String>, label: impl Into<Label>) -> Param {
    let key = key.into();
    let name = key.clone();
    Param::custom(key, label, move |ui, fields, _| {
        let (response, next) = tri_box(ui, fields.bool(&name), None, &name);
        match next {
            Some(Some(on)) => fields.set(name.clone(), FieldValue::Bool(on)),
            Some(None) => {
                fields.remove(&name);
            }
            None => {}
        }
        response
    })
    // Set from outside: on, off, or `null` for neither.
    .on_set(|fields, key, value| {
        match value {
            Value::Bool(on) => fields.set(key, FieldValue::Bool(on)),
            Value::Null => {
                fields.remove(key);
            }
            other => return Err(FieldRefusal::Rejects(&other).message(key)),
        }
        Ok(())
    })
}

/// The General tab's rows: the style's name and what it sets, or nothing of that while `attrs` is refused.
pub(crate) fn general_rows(attrs: Result<Map<String, Value>, String>) -> Vec<PropertyRow> {
    let mut rows = vec![PropertyRow::field(text("name", tl!("Style Name:")))];
    rows.extend(attrs.into_iter().flatten().map(|(key, value)| PropertyRow::value(Label::Empty, super::form::attribute_line(&key, &value))));
    rows
}

/// A refusal's text as the dialog shows it under the rows.
pub(crate) fn reason(why: &str) -> Label {
    Label::from(crate::i18n::message(crate::i18n::current(), why))
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

/// Is the feature a tag of a style's list sets on? A `tag=N` of a valued feature is off for 0,
/// which a `-` prefix check misses; every other tag is on unless it starts with `-`.
fn tag_on(tag: &str) -> bool {
    let name = bare(tag);
    let valued = name == "swsh" || vectorcraft_text::EXTRA_FEATURES.contains(&name);
    if valued && vectorcraft_text::OtFeatures::known_tag(tag) {
        vectorcraft_text::OtFeatures::from_tags([tag]).feature_value(name).is_some()
    } else {
        !tag.starts_with('-')
    }
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
        fields.insert((*field).into(), last.map_or(Value::Null, |t| json!(tag_on(t))));
    }
    let last_on = |tag: &str| tags.iter().rev().find(|t| bare(t) == tag).map(|t| tag_on(t));
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
        .filter_map(|set| tags.iter().rev().find(|t| bare(t) == *set).map(|t| if tag_on(t) { set.to_string() } else { format!("-{set}") }))
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

fn optional_number(d: &Values, key: &str) -> Result<Option<Value>, String> {
    let label = match key {
        "size" => tl!("Size"),
        "leading" => tl!("Leading"),
        "kerning" => tl!("Kerning"),
        "tracking" => tl!("Tracking"),
        "h_scale" => tl!("Horizontal Scale"),
        "v_scale" => tl!("Vertical Scale"),
        "baseline_shift" => tl!("Baseline Shift"),
        "tcyUpDown" => tl!("Tate-chu-yoko Up/Down"),
        "tcyLeftRight" => tl!("Tate-chu-yoko Left/Right"),
        "rotation" => tl!("Character Rotation"),
        "akiLeft" => tl!("Insert Aki Left"),
        "akiRight" => tl!("Insert Aki Right"),
        "tsume" => tl!("Tsume"),
        "diacriticHorizontal" => tl!("Adjust Horizontal Diacritic Position"),
        "diacriticVertical" => tl!("Adjust Vertical Diacritic Position"),
        "fill_tint" => tl!("Fill Tint"),
        "stroke_tint" => tl!("Stroke Tint"),
        "stroke_width" => tl!("Weight"),
        _ => return Err("unknown character style number field".into()),
    };
    match d.get(key) {
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
            if ["baseline_shift", "tcyUpDown", "tcyLeftRight"].contains(&key) && !(-1296.0..=1296.0).contains(&n) {
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
            // Insert Aki stops at 0 and at the largest value the style takes, as the Character
            // panel's fields do.
            let n = if AKI.contains(&key) { n.clamp(0.0, vectorcraft_doc::text::MAX_AKI) } else { n };
            // Tsume stops at 0% and 100% the same way.
            let n = if key == "tsume" { n.clamp(0.0, vectorcraft_doc::text::MAX_TSUME) } else { n };
            // The diacritic positions stop at the largest value the style takes either way.
            let max = vectorcraft_doc::text::MAX_DIACRITIC_POSITION;
            let n = if DIACRITIC.contains(&key) { n.clamp(-max, max) } else { n };
            Ok(Some(json!(n)))
        }
    }
}

/// The style's attributes as the dialog's `fields` set them. A field still as it opened keeps its
/// stored value without a check, so a style read from a file with a value out of range can still be
/// renamed; a tab that was reset writes its unset fields as unset.
pub(crate) fn edited_attrs(fields: &DialogFields, opening: &Opening) -> Result<Map<String, Value>, String> {
    let d = Values::new(fields);
    let original = &opening.attrs;
    let original_fields = canonical_map(fields_from_attrs(original));
    let reset = d.reset("basic");
    let mut attrs = original.clone();
    for key in BASIC {
        attrs.remove(*key);
    }
    for key in ["font_family", "font_style"] {
        if let Some(v) = d.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            attrs.insert(key.into(), json!(v.trim()));
        }
    }
    if reset || ["font_family", "font_style"].iter().any(|key| d.get(key) != original_fields.get(*key)) {
        attrs.remove("font_version");
        attrs.remove("fontAxes");
    }
    for key in NUMBERS {
        // An unchanged field keeps the original value below, unchecked (see `advanced_attrs`).
        if !reset && d.get(key) == original_fields.get(*key) {
            continue;
        }
        if let Some(v) = optional_number(&d, key)? {
            attrs.insert((*key).into(), v);
        }
    }
    for key in SWITCHES {
        if let Some(v) = d.get(key).and_then(Value::as_bool) {
            attrs.insert((*key).into(), json!(v));
        }
    }
    if reset || d.get("case").and_then(Value::as_str) != case_from_attrs(original) {
        match d.get("case").and_then(Value::as_str) {
            Some("allCaps") => {
                attrs.insert("all_caps".into(), json!(true));
                attrs.insert("small_caps".into(), Value::Null);
            }
            Some("smallCaps") => {
                attrs.insert("all_caps".into(), json!(false));
                attrs.insert(
                    "small_caps".into(),
                    original.get("small_caps").filter(|v| v.is_number()).cloned().unwrap_or_else(|| json!(opening.small_caps_size)),
                );
            }
            Some("normal") => {
                attrs.insert("all_caps".into(), json!(false));
                attrs.insert("small_caps".into(), Value::Null);
            }
            _ => {}
        }
    }
    if let Some(position) = d.get("position").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        let value = if position == "normal" {
            json!({"kind": "normal"})
        } else {
            original.get("position").filter(|v| v.get("kind").and_then(Value::as_str) == Some(position)).cloned().unwrap_or_else(|| {
                let [size, at] = if position == "superscript" { opening.superscript } else { opening.subscript };
                json!({"kind": position, "size": size, "position": at})
            })
        };
        attrs.insert("position".into(), value);
    }
    if let Some(align) = d.get("charAlign").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        attrs.insert("charAlign".into(), json!(align));
    }
    if !reset {
        for key in BASIC {
            let field = if ["all_caps", "small_caps"].contains(key) { "case" } else { key };
            if d.get(field) == original_fields.get(field) {
                attrs.remove(*key);
                if let Some(value) = original.get(*key) {
                    attrs.insert((*key).into(), value.clone());
                }
            }
        }
    }
    advanced_attrs(&d, original, &original_fields, &mut attrs)?;
    color_attrs(&d, original, &original_fields, &mut attrs)?;
    opentype_attrs(&d, original, &original_fields, &mut attrs);
    Ok(attrs)
}

/// Write the Character Color fields of `d` into `attrs`: an unset paint, weight or checkbox
/// leaves its attribute unset, and a field still as it opened keeps the original value, a
/// gradient or pattern paint included.
fn color_attrs(
    d: &Values,
    original: &Map<String, Value>,
    original_fields: &Map<String, Value>,
    attrs: &mut Map<String, Value>,
) -> Result<(), String> {
    let reset = d.reset("color");
    for key in COLOR {
        attrs.remove(*key);
    }
    let unchanged = |key: &str| !reset && d.get(key) == original_fields.get(key);
    for key in ["fill", "stroke"] {
        let tint_key = format!("{key}_tint");
        if unchanged(key) && unchanged(&tint_key) {
            if let Some(value) = original.get(key) {
                attrs.insert(key.into(), value.clone());
            }
            continue;
        }
        let Some(mut paint) = d.get(key).filter(|v| v.is_object()).cloned() else { continue };
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
        let value = if unchanged(key) { original.get(key).cloned() } else { d.get(key).filter(|v| v.is_boolean()).cloned() };
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
fn opentype_attrs(d: &Values, original: &Map<String, Value>, original_fields: &Map<String, Value>, attrs: &mut Map<String, Value>) {
    let reset = d.reset("opentype");
    attrs.remove("features");
    let unchanged = |key: &str| !reset && d.get(key) == original_fields.get(key);
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
            if let Some(on) = d.get(field).and_then(Value::as_bool) {
                tags.push(if on { (*tag).to_string() } else { format!("-{tag}") });
            }
        }
    }
    if !unchanged("ot_figure") {
        tags.retain(|t| !FIGURE_TAGS.contains(&bare(t)));
        tags.extend(figure_tags(d.str("ot_figure")).iter().map(|t| t.to_string()));
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
        let before: Vec<String> = tags.iter().filter(|t| sets.contains(&bare(t))).cloned().collect();
        tags.retain(|t| !sets.contains(&bare(t)));
        let listed = d.get("ot_sets").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        tags.extend(listed.filter(|t| sets.contains(&bare(t))).map(|t| {
            let kept = before
                .iter()
                .rev()
                .find(|b| bare(b) == bare(t))
                .filter(|b| !t.starts_with('-') && vectorcraft_text::OtFeatures::known_tag(b) && tag_on(b));
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
    d: &Values,
    original: &Map<String, Value>,
    original_fields: &Map<String, Value>,
    attrs: &mut Map<String, Value>,
) -> Result<(), String> {
    let reset = d.reset("advanced");
    for key in ADVANCED {
        attrs.remove(*key);
    }
    // A field still as it opened keeps its original value below without a check.
    let unchanged = |key: &str| !reset && d.get(key) == original_fields.get(key);
    for key in ADVANCED_NUMBERS {
        if unchanged(key) {
            continue;
        }
        if let Some(v) = optional_number(d, key)? {
            attrs.insert((*key).into(), v);
        }
    }
    if !unchanged("language")
        && let Some(language) = d.get("language").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
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

/// The swatches of a [`swatch_choices`] value as (name, paint).
fn swatch_pairs(swatches: &Value) -> Vec<(String, Value)> {
    swatches.as_array().into_iter().flatten().filter_map(|pair| Some((pair.get(0)?.as_str()?.to_string(), pair.get(1)?.clone()))).collect()
}

/// Keeps the flags the tint and weight rows read in step with the paints: whether each paint links to a swatch
/// (`fill_linked`, `stroke_linked`) and whether the stroke is None (`stroke_none`). Writes only what differs.
fn paint_flags(fields: &mut DialogFields) {
    let paint = |fields: &DialogFields, key: &str| fields.get(key).map(value_of).unwrap_or(Value::Null);
    let mut flags: Vec<(&str, bool)> = ["fill", "stroke"]
        .into_iter()
        .map(|key| (if key == "fill" { "fill_linked" } else { "stroke_linked" }, paint(fields, key).get("swatch").is_some_and(Value::is_string)))
        .collect();
    flags.push(("stroke_none", paint(fields, "stroke").get("type").and_then(Value::as_str) == Some("none")));
    for (key, on) in flags {
        if fields.bool(key) != Some(on) {
            fields.set(key, FieldValue::Bool(on));
        }
    }
}

/// The id of the menu entry that shows `paint`: empty for unset, `none`, `swatch:NAME` for a swatch's paint or
/// another tint of its swatch, else `current` for a paint no swatch gives, which stays as it is until another
/// entry is chosen.
fn paint_id(paint: &Value, swatches: &[(String, Value)]) -> String {
    if paint.is_null() {
        return String::new();
    }
    if paint.get("type").and_then(Value::as_str) == Some("none") {
        return "none".into();
    }
    let by_paint = swatches.iter().find(|(_, p)| p == paint);
    let linked = || swatches.iter().find(|(_, p)| p.get("swatch").is_some_and(|s| Some(s) == paint.get("swatch")));
    by_paint.or_else(linked).map_or_else(|| "current".into(), |(name, _)| format!("swatch:{name}"))
}

/// The Fill or Stroke menu `key`: unset, None, or a swatch of the document. Choosing an entry also sets the tint
/// the paint gives, as the Tint row shows it.
fn paint_row(key: &'static str, label: &'static str, swatches: Vec<(String, Value)>) -> Param {
    let companions: &[&str] = if key == "fill" { &["fill_linked"] } else { &["stroke_linked", "stroke_none"] };
    Param::custom(key, label, move |ui, fields, _| {
        let current = fields.get(key).map(value_of).unwrap_or(Value::Null);
        let selected = paint_id(&current, &swatches);
        let mut entries = vec![Entry::new(String::new(), Label::from(" ")), Entry::new("none".to_string(), Label::from(tl!("None")))];
        entries.extend(swatches.iter().map(|(name, _)| Entry::new(format!("swatch:{name}"), Label::from(name.clone()))));
        if selected == "current" {
            let shown = serde_json::from_value::<vectorcraft_color::Paint>(current.clone()).map(|p| p.label()).unwrap_or_default();
            entries.push(Entry::new(selected.clone(), Label::from(shown)));
        }
        let out = Select::new(Label::from(label), Bind::given(selected), &entries).show(ui);
        if let Some(id) = out.value().cloned() {
            let paint = match id.as_str() {
                "" => Some(Value::Null),
                "none" => Some(json!({"type": "none"})),
                other => other.strip_prefix("swatch:").and_then(|name| swatches.iter().find(|(n, _)| n == name)).map(|(_, paint)| paint.clone()),
            };
            if let Some(paint) = paint {
                for (field_key, value) in [(key.to_string(), paint.clone()), (format!("{key}_tint"), tint_field(&paint))] {
                    match field_of(&value) {
                        Some(value) => fields.set(field_key, value),
                        None => {
                            fields.remove(&field_key);
                        }
                    }
                }
            }
        }
        paint_flags(fields);
        out.response().clone()
    })
    .companions(companions.iter().copied())
    .on_set(|fields, key, value| {
        match field_of(&value) {
            Some(value) => fields.set(key, value),
            None => {
                fields.remove(key);
            }
        }
        paint_flags(fields);
        Ok(())
    })
}

/// The Stylistic Sets grid: each set is on (`ssNN`), off (`-ssNN`) or unset (not listed), as the other checkboxes are.
fn sets_row() -> Param {
    Param::custom("ot_sets", tl!("Stylistic Sets"), |ui, fields, _| {
        let mut listed: Vec<String> = fields
            .get("ot_sets")
            .map(value_of)
            .and_then(|v| v.as_array().map(|tags| tags.iter().filter_map(Value::as_str).map(str::to_string).collect()))
            .unwrap_or_default();
        let mut clicked = None;
        let line = ui.horizontal_wrapped(|ui| {
            for set in &vectorcraft_text::EXTRA_FEATURES[..20] {
                let current = listed.iter().find(|t| bare(t) == *set).map(|t| !t.starts_with('-'));
                if let (_, Some(next)) = tri_box(ui, current, Some(Label::from(set.to_string())), set) {
                    clicked = Some((*set, next));
                }
            }
        });
        if let Some((set, next)) = clicked {
            listed.retain(|t| bare(t) != set);
            match next {
                Some(true) => listed.push(set.to_string()),
                Some(false) => listed.push(format!("-{set}")),
                None => {}
            }
            if listed.is_empty() {
                fields.remove("ot_sets");
            } else {
                fields.set("ot_sets", FieldValue::Json(json!(listed)));
            }
        }
        line.response
    })
}

fn basic_rows() -> Vec<PropertyRow> {
    let case = [("normal", tl!("Normal")), ("allCaps", tl!("All Caps")), ("smallCaps", tl!("Small Caps"))];
    let position = [("normal", tl!("Normal")), ("superscript", tl!("Superscript")), ("subscript", tl!("Subscript"))];
    let align = [
        ("romanBaseline", tl!("Roman Baseline")),
        ("emBoxTop", tl!("Em Box Top/Right")),
        ("emBoxCenter", tl!("Em Box Center")),
        ("emBoxBottom", tl!("Em Box Bottom/Left")),
        ("icfTop", tl!("ICF Top/Right")),
        ("icfBottom", tl!("ICF Bottom/Left")),
    ];
    rows(vec![
        text("font_family", tl!("Font Family:")),
        text("font_style", tl!("Font Style:")),
        text("size", tl!("Size")),
        text("leading", tl!("Leading")),
        text("kerning", tl!("Kerning")),
        text("tracking", tl!("Tracking")),
        select("case", tl!("Case"), &case),
        select("position", tl!("Position"), &position),
        tri_state("underline", tl!("Underline")),
        tri_state("strikethrough", tl!("Strikethrough")),
        tri_state("standardVerticalRoman", tl!("Standard Vertical Roman Alignment")),
        select("charAlign", tl!("Character Alignment"), &align),
    ])
}

// PLAN: the Aki, Tsume and Diacritic rows are plain text rows: `Param::text` has no preset menu to carry the Character panel's presets.
fn advanced_rows() -> Vec<PropertyRow> {
    let mut params: Vec<Param> = [
        ("h_scale", tl!("Horizontal Scale"), "%"),
        ("v_scale", tl!("Vertical Scale"), "%"),
        ("baseline_shift", tl!("Baseline Shift"), "pt"),
        ("tcyUpDown", tl!("Tate-chu-yoko Up/Down"), "pt"),
        ("tcyLeftRight", tl!("Tate-chu-yoko Left/Right"), "pt"),
        ("rotation", tl!("Character Rotation"), "°"),
        ("akiLeft", tl!("Insert Aki Left"), ""),
        ("akiRight", tl!("Insert Aki Right"), ""),
        ("tsume", tl!("Tsume"), "%"),
        ("diacriticHorizontal", tl!("Adjust Horizontal Diacritic Position"), ""),
        ("diacriticVertical", tl!("Adjust Vertical Diacritic Position"), ""),
    ]
    .into_iter()
    .map(|(key, caption, unit)| text(key, if unit.is_empty() { caption.to_string() } else { format!("{caption} ({unit})") }))
    .collect();
    let languages: Vec<(&'static str, &'static str)> = vectorcraft_doc::setup::LANGUAGES.iter().map(|(name, _, _)| (*name, *name)).collect();
    params.push(select("language", tl!("Language:"), &languages));
    rows(params)
}

fn color_rows(opening: &Opening) -> Vec<PropertyRow> {
    let swatches = swatch_pairs(&opening.swatches);
    rows(vec![
        paint_row("fill", tl!("Fill"), swatches.clone()),
        text("fill_tint", tl!("Fill Tint")).enabled_when("fill_linked", FieldValue::Bool(true)),
        paint_row("stroke", tl!("Stroke"), swatches),
        text("stroke_tint", tl!("Stroke Tint")).enabled_when("stroke_linked", FieldValue::Bool(true)),
        text("stroke_width", tl!("Weight")).enabled_when("stroke_none", FieldValue::Bool(false)),
        tri_state("overprint_fill", tl!("Overprint Fill")),
        tri_state("overprint_stroke", tl!("Overprint Stroke")),
    ])
}

fn opentype_rows() -> Vec<PropertyRow> {
    let figures: Vec<(&'static str, &'static str)> = FIGURE_VALUES.into_iter().zip(crate::panels::opentype::FIGURES).collect();
    let positions: Vec<(&'static str, &'static str)> = POSITION_VALUES.into_iter().zip(crate::panels::opentype::POSITIONS).collect();
    let captions = [
        tl!("Standard Ligatures"),
        tl!("Contextual Alternates"),
        tl!("Discretionary Ligatures"),
        tl!("Swash"),
        tl!("Stylistic Alternates"),
        tl!("Titling Alternates"),
        tl!("Ordinals"),
        tl!("Fractions"),
        tl!("H or V Style"),
        tl!("Roman Italics"),
    ];
    let mut params = vec![select("ot_figure", tl!("Figure:"), &figures), select("ot_position", tl!("Position:"), &positions)];
    params.extend(OT_SWITCHES.iter().zip(captions).map(|((key, _), caption)| tri_state(*key, caption)));
    let mut rows = rows(params);
    rows.push(PropertyRow::heading(tl!("Stylistic Sets")));
    rows.push(PropertyRow::field(sets_row()));
    rows
}

/// The five tabs of Character Style Options and New Character Style over the dialog's current values. Every tab
/// but General has a Reset Panel.
pub(crate) fn tabs(fields: &DialogFields, opening: &Opening) -> Vec<PropertiesTab> {
    let titles =
        [tl!("General"), tl!("Basic Character Formats"), tl!("Advanced Character Formats"), tl!("Character Color"), tl!("OpenType Features")];
    let bodies = [general_rows(edited_attrs(fields, opening)), basic_rows(), advanced_rows(), color_rows(opening), opentype_rows()];
    SECTIONS
        .into_iter()
        .zip(titles)
        .zip(bodies)
        .map(|((id, title), rows)| {
            let tab = PropertiesTab::new(title, rows).key(id);
            if id == "general" { tab } else { tab.resettable() }
        })
        .collect()
}

/// The dialog of style `name` over `opening`, which `title` heads, kept open by a held OK.
pub(crate) fn hosted(key: &str, title: &'static str, opening: &Arc<Opening>, name: &str) -> HostedPropertiesDialog {
    let rows_of = Arc::clone(opening);
    HostedPropertiesDialog::new(key, opening.fields(name), move |values| {
        let now = values.value().cloned().unwrap_or_else(DialogFields::new);
        PropertiesDialog::new(title, tabs(&now, &rows_of), values).tab_layout(TabLayout::List)
    })
    .held_ok()
}

/// What a style dialog's pump reads of one frame.
pub(crate) struct Frame {
    /// The command's parameters for the values, or why they are refused.
    pub(crate) params: Result<Value, String>,
    /// Whether the dialog previews and its Preview check box is on.
    pub(crate) previewing: bool,
    /// OK was pressed this frame and waits for the answer.
    pub(crate) ok: bool,
}

/// Runs one frame of an options dialog's live preview of `cmd`: previews the values while they are accepted, keeps
/// the last accepted preview while they are refused, and at OK keeps the preview as one undo step. Returns the
/// preview state to keep and the commit's answer at OK.
pub(crate) fn drive(
    app: &mut VectorcraftApp,
    mut live: LivePreview,
    label: &str,
    cmd: &str,
    frame: &Frame,
) -> (LivePreview, Option<Result<Value, String>>) {
    match (&frame.params, frame.ok) {
        (Ok(params), true) => (LivePreview::default(), Some(live.commit(app, cmd, params.clone()))),
        (Err(why), true) => (live, Some(Err(why.clone()))),
        (Ok(params), false) => {
            live.run(app, frame.previewing, label, cmd, params);
            (live, None)
        }
        // Refused values keep the last accepted preview; turning Preview off still rolls it back.
        (Err(_), false) => {
            if !frame.previewing {
                live.run(app, false, label, cmd, &Value::Null);
            }
            (live, None)
        }
    }
}

/// Sets `field` of the style dialog `inner` from automation: a number a text row refuses is taken as its text, as
/// typing it would give.
pub(crate) fn set_field(inner: &mut HostedPropertiesDialog, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
    use craft::ui::dialog::StackDialog as _;
    let refused = match inner.set_json_field(ctx, field, value.clone()) {
        Err(why) if value.is_number() => why,
        done => return done,
    };
    let text = field_of(&value).map(|typed| value_of(&typed)).unwrap_or(Value::Null);
    inner.set_json_field(ctx, field, text).map_err(|_| refused)
}

/// Makes a wrapper over a [`HostedPropertiesDialog`] (`inner`) a dialog of its own type: it answers with the values
/// and delegates everything else to `inner`. The wrapper has `edited()` and `command(attrs)` of its own; `frame` is
/// what its pump reads each frame, which also keeps OK off while `edited()` refuses.
macro_rules! style_dialog {
    ($dialog:ty, $kind:expr) => {
        impl $dialog {
            /// The hosted dialog, for Reset Panel and the pump's answers.
            pub(crate) fn hosted_mut(&mut self) -> &mut craft::ui::patterns::form_dialogs::HostedPropertiesDialog {
                &mut self.inner
            }

            /// This frame's state: the refusal is shown under the rows and OK is taken if it was pressed.
            pub(crate) fn frame(&mut self) -> $crate::dialogs::char_style_options::Frame {
                let edited = self.edited();
                self.inner.refuse(edited.as_ref().err().map(|why| $crate::dialogs::char_style_options::reason(why)));
                $crate::dialogs::char_style_options::Frame {
                    previewing: self.inner.is_previewing(),
                    ok: self.inner.take_ok().is_some(),
                    params: edited.map(|attrs| self.command(attrs)),
                }
            }
        }

        impl craft::ui::dialog::StackDialog for $dialog {
            type Result = craft::ui::patterns::form_dialogs::DialogFields;

            fn id(&self) -> craft::ui::dialog::DialogId {
                craft::ui::dialog::StackDialog::id(&self.inner)
            }

            fn show(&mut self, ui: &mut egui::Ui, stack: &craft::ui::dialog::DialogStack) -> Option<craft::ui::dialog::DialogResult<Self::Result>> {
                craft::ui::dialog::StackDialog::show(&mut self.inner, ui, stack)
            }

            fn key(&self) -> Option<&str> {
                Some($kind)
            }

            fn confirm(&mut self, ctx: &egui::Context) -> Option<Self::Result> {
                craft::ui::dialog::StackDialog::confirm(&mut self.inner, ctx)
            }

            fn held_state(&self) -> Option<craft::ui::dialog::HeldOk> {
                craft::ui::dialog::StackDialog::held_state(&self.inner)
            }

            fn json_fields(&self) -> Option<serde_json::Value> {
                craft::ui::dialog::StackDialog::json_fields(&self.inner)
            }

            fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
                $crate::dialogs::char_style_options::set_field(&mut self.inner, ctx, field, value)
            }

            fn cancelled(&mut self, ctx: &egui::Context) {
                craft::ui::dialog::StackDialog::cancelled(&mut self.inner, ctx);
            }

            fn can_reset(&self) -> bool {
                self.inner.can_reset_tab()
            }

            fn reset(&mut self, _ctx: &egui::Context) -> bool {
                self.inner.reset_tab()
            }
        }
    };
}
pub(crate) use style_dialog;

/// The open Character Style Options dialog.
pub(crate) struct CharStyleOptions {
    inner: HostedPropertiesDialog,
    opening: Arc<Opening>,
}

impl CharStyleOptions {
    /// The style's attributes as the dialog's values set them, or why they are refused.
    pub(crate) fn edited(&self) -> Result<Map<String, Value>, String> {
        edited_attrs(self.inner.fields(), &self.opening)
    }

    /// What the dialog opened with.
    pub(crate) fn opening(&self) -> &Opening {
        &self.opening
    }

    fn command(&self, attrs: Map<String, Value>) -> Value {
        let to = self.inner.fields().text("name").unwrap_or_default();
        json!({"name": self.opening.style, "to": to, "attrs": attrs})
    }
}
style_dialog!(CharStyleOptions, KIND);

pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    crate::panels::character::end_typing(app);
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let def = doc.char_styles.iter().find(|s| s.name == name).ok_or_else(|| format!("no character style `{name}`"))?;
    let opening = Arc::new(Opening::new(name, def.attrs.clone(), &doc.setup, swatch_choices(doc)));
    let dialog = CharStyleOptions { inner: hosted(KIND, tl!("Character Style Options"), &opening, name).previews(), opening };
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(json!({"dialog": KIND}))
}

/// Reset Panel: clears the settings of the tab the open Character Style Options or New Character Style dialog
/// shows.
pub fn reset_panel(app: &mut VectorcraftApp) -> Result<Value, String> {
    let dialogs = app.dialogs().ok_or("no dialog open")?;
    let reset = dialogs.with(|host| {
        let options = host.get_mut::<CharStyleOptions>().map(|d| (KIND, d.hosted_mut().reset_tab()));
        options.or_else(|| {
            let new = host.get_mut::<super::new_text_style::NewTextStyle>().filter(|d| !d.para);
            new.map(|d| (super::new_text_style::KIND, d.hosted_mut().reset_tab()))
        })
    });
    match reset {
        Some((kind, true)) => Ok(json!({"dialog": kind})),
        _ => Err("the open dialog has no Reset".into()),
    }
}

/// The dialog's frame work: previews the values, commits at OK, and rolls the preview back at Cancel.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(frame) = dialogs.with(|host| host.get_mut::<CharStyleOptions>().map(CharStyleOptions::frame)) else {
        if matches!(dialogs.with(|host| host.take::<CharStyleOptions>()), Some(DialogResult::Cancel)) {
            std::mem::take(&mut app.char_style_preview).cancel(app);
        }
        return;
    };
    let live = std::mem::take(&mut app.char_style_preview);
    let (live, done) = drive(app, live, tl!("Character Style Options"), "charStyle.edit", &frame);
    app.char_style_preview = live;
    if let Some(done) = done {
        let answer = done.map(drop).map_err(|why| reason(&why));
        dialogs.with(|host| {
            if let Some(d) = host.get_mut::<CharStyleOptions>() {
                d.hosted_mut().answer_ok(answer);
            }
        });
    }
}

#[cfg(test)]
mod tests;
