//! Paragraph Style Options fields shared by editing and creating a paragraph style: General, Basic
//! Paragraph Formats (alignment, indents and spacing), Tabs (the style's tab stops), Composition
//! (composer and Roman Hanging Punctuation), Hyphenation, Justification and East Asian Composition
//! Settings (Bunri-Kinshi and Korean Line Break Anywhere), as the tabs of a hosted properties dialog.
//!
//! Each field is unset (empty, absent, or a checkbox in its unset state) or set, and an unset field leaves
//! its attribute out of the style. Distances are shown and typed in the General unit and kept in
//! points. The Hyphenation and Justification options are one attribute each (`hyphenation`,
//! `justification`): when any of their fields is set, a field left empty takes its default value.
//! The `tabs` field is unset (absent) or a list of stops, an empty list included, and is the `tabs`
//! attribute. A field still as it opened keeps the style's stored value as it is.
//!
//! Behavior reference (public documentation):
//! <https://helpx.adobe.com/illustrator/using/character-paragraph-styles.html>.

use std::sync::Arc;

use craft::ui::buttons::Button;
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::DialogResult;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedPropertiesDialog, Param, ParamKind, PropertiesTab, PropertyRow};
use craft::ui::runtime::env::Label;
use serde_json::{Map, Value, json};
use vectorcraft_doc::text::DEFAULT_TAB_INTERVAL;
use vectorcraft_doc::{HyphenationOptions, JustificationOptions, TabAlign, TabStop, Unit};

use super::char_style_options::{
    Values, canonical_map, drive, general_rows, reason, rows, select, style_dialog, text, to_fields, tri_state, value_of,
};
use crate::VectorcraftApp;

pub const KIND: &str = "paraStyleOptions";

/// The section ids, in the order the tabs show them.
pub(crate) const SECTIONS: [&str; 7] = ["general", "basic", "tabs", "composition", "hyphenation", "justification", "eastAsian"];
/// The alignments of a tab stop with the names the `tabs` field gives them, in the Tabs panel's order.
const TAB_ALIGNS: [(TabAlign, &str); 4] =
    [(TabAlign::Left, "left"), (TabAlign::Center, "center"), (TabAlign::Right, "right"), (TabAlign::Decimal, "decimal")];
/// The distance fields of Basic Paragraph Formats, which are also the attributes they set.
const DISTANCES: [&str; 5] = ["left_indent", "right_indent", "first_line_indent", "space_before", "space_after"];
/// The range of the indents, the space before and after, and the Hyphenation Zone (pt), as
/// `text.setFormat` keeps them.
const MAX_DISTANCE: f64 = 1296.0;
/// The whole-number fields of Hyphenation: (field, `hyphenation` key, range).
const COUNTS: [(&str, &str, (f64, f64)); 4] = [
    ("words_longer_than", "words_longer_than", (1.0, 25.0)),
    ("after_first", "after_first", (1.0, 15.0)),
    ("before_last", "before_last", (1.0, 15.0)),
    ("hyphen_limit", "hyphen_limit", (0.0, 25.0)),
];
/// The fields of Hyphenation that make up the `hyphenation` attribute.
const HYPHENATION_FIELDS: [&str; 6] =
    ["words_longer_than", "after_first", "before_last", "hyphen_limit", "hyphenation_zone", "hyphenate_capitalized"];
/// The spacing rows of Justification: (field prefix, `justification` key, range).
const ROWS: [(&str, &str, (f64, f64)); 3] = [
    ("word", "word_spacing", JustificationOptions::WORD_SPACING),
    ("letter", "letter_spacing", JustificationOptions::LETTER_SPACING),
    ("glyph", "glyph_scaling", JustificationOptions::GLYPH_SCALING),
];
/// The columns of a spacing row: Minimum, Desired and Maximum.
const COLUMNS: [&str; 3] = ["min", "desired", "max"];
/// The fields of Justification that make up the `justification` attribute.
const JUSTIFICATION_FIELDS: [&str; 11] = [
    "word_min",
    "word_desired",
    "word_max",
    "letter_min",
    "letter_desired",
    "letter_max",
    "glyph_min",
    "glyph_desired",
    "glyph_max",
    "auto_leading",
    "single_word",
];
/// The values of the Alignment field (`justify`), in the Paragraph panel's order after Auto.
const JUSTIFY_VALUES: [&str; 8] = ["Auto", "Left", "Center", "Right", "JustifyLeft", "JustifyCenter", "JustifyRight", "JustifyAll"];
/// The values of the Single Word Justification field.
const SINGLE_WORD_VALUES: [&str; 4] = ["fullJustify", "alignLeft", "alignCenter", "alignRight"];
/// The fields that edit the selected tab stop, with the `tabs` entry key each one shows.
const TAB_EDITORS: [(&str, &str); 4] =
    [("__tabAlign", "align"), ("__tabPosition", "position"), ("__tabLeader", "leader"), ("__tabAlignOn", "alignOn")];

/// What a paragraph style dialog opened with.
pub(crate) struct Opening {
    /// The style's name when the dialog opened; empty for a new style.
    pub(crate) style: String,
    /// The style's attributes when the dialog opened.
    pub(crate) attrs: Map<String, Value>,
    /// The unit distances are shown and typed in.
    pub(crate) unit: Unit,
    /// Whether the selected type is vertical, where Tabs names a stop's position Y.
    pub(crate) vertical: bool,
}

impl Opening {
    /// The opening of style `style` with `attrs`, its distances shown in `unit`.
    pub(crate) fn new(style: &str, attrs: Map<String, Value>, unit: Unit, vertical: bool) -> Opening {
        Opening { style: style.into(), attrs, unit, vertical }
    }

    /// The dialog's values at opening, for a style called `name`.
    pub(crate) fn fields(&self, name: &str) -> DialogFields {
        let mut fields = form_fields(&self.attrs, self.unit);
        fields.set("name", FieldValue::Text(name.into()));
        fields
    }
}

/// The dialog's values for a style's `attrs` in `unit`.
pub(crate) fn form_fields(attrs: &Map<String, Value>, unit: Unit) -> DialogFields {
    to_fields(&fields_from_attrs(attrs, unit))
}

/// Whether the first selected text is vertical type, where the Tabs section names a stop's
/// position Y and its Left and Right alignments Top and Bottom.
pub(crate) fn vertical_text(app: &VectorcraftApp) -> bool {
    crate::panels::character::first_selected_text(app).is_some_and(|t| t.vertical)
}

/// The `tabs` field for a style's `tabs` attribute: unset (null) when the style leaves it out,
/// otherwise one entry per stop ([`stop_field`]).
fn tabs_field(value: Option<&Value>, unit: Unit) -> Value {
    let Some(stops) = value.and_then(Value::as_array) else { return Value::Null };
    Value::Array(stops.iter().filter_map(|s| serde_json::from_value::<TabStop>(s.clone()).ok()).map(|s| stop_field(&s, unit)).collect())
}

/// A tab stop's entry in the `tabs` field: its position as text in `unit`, its alignment, leader
/// and Align On character, and the position in points it shows (`__pt`), which a position still as
/// it shows keeps exactly.
fn stop_field(s: &TabStop, unit: Unit) -> Value {
    let align = TAB_ALIGNS.iter().find(|(a, _)| *a == s.align).map_or("left", |(_, key)| *key);
    json!({"position": unit.format(s.position), "align": align, "leader": s.leader, "alignOn": s.align_on.to_string(), "__pt": s.position})
}

/// The position in points of a `tabs` entry: its `__pt` while its text is as that shows in `unit`,
/// else the text read in `unit`; `None` when the text is not a distance.
fn stop_position(row: &Value, unit: Unit) -> Option<f64> {
    let text = row.get("position").and_then(Value::as_str).map_or("", str::trim);
    row.get("__pt").and_then(Value::as_f64).filter(|pt| pt.is_finite() && unit.format(*pt) == text).or_else(|| unit.parse(text))
}

/// The tab stop a `tabs` entry sets, or why it sets none: its position must be a distance from 0 to
/// 100000 pt (`label` names the field), its alignment one of [`TAB_ALIGNS`], its leader text of at
/// most eight characters and its Align On character one character. A leader or Align On character
/// left out is none or a period.
fn read_stop(row: &Value, unit: Unit, label: &str) -> Result<TabStop, String> {
    let position = stop_position(row, unit).ok_or_else(|| format!("{label} must be a number"))?;
    if !(0.0..=TabStop::MAX_POSITION).contains(&position) {
        let (min, max) = (unit.format(0.0), unit.format(TabStop::MAX_POSITION));
        return Err(format!("{label} must be a number from {min} to {max}"));
    }
    let other = match row.get("align") {
        None | Some(Value::Null) => "left",
        Some(v) => v.as_str().unwrap_or(""),
    };
    let align = TAB_ALIGNS.iter().find(|(_, key)| *key == other).map(|(a, _)| *a).ok_or_else(|| format!("unknown align `{other}`"))?;
    let leader = match row.get("leader") {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => {
            let key = "leader";
            return Err(format!("`{key}` must be a string"));
        }
    };
    let (label, max) = (caption(tl!("Leader:")), TabStop::MAX_LEADER);
    if leader.chars().count() > max {
        return Err(format!("{label} must be at most {max} characters"));
    }
    let label = caption(tl!("Align On:"));
    let align_on = match row.get("alignOn") {
        None | Some(Value::Null) => '.',
        Some(Value::String(s)) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => c,
                _ => return Err(format!("{label} must be one character")),
            }
        }
        Some(_) => return Err(format!("{label} must be one character")),
    };
    Ok(TabStop { position, align, leader: leader.into(), align_on })
}

/// A distance in points as its field shows it in `unit`.
fn distance_field(value: Option<&Value>, unit: Unit) -> Value {
    value.and_then(Value::as_f64).map_or_else(|| json!(""), |pt| json!(unit.format(pt)))
}

/// A number as its field shows it, or empty.
fn number_field(value: Option<&Value>) -> Value {
    value.and_then(Value::as_f64).map_or_else(|| json!(""), |n| json!(format_number(n)))
}

fn format_number(n: f64) -> String {
    let s = format!("{n:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// The fields for a style's `attrs` in `unit`: an attribute the style does not set is an empty
/// field or an unset checkbox or menu.
pub(crate) fn fields_from_attrs(attrs: &Map<String, Value>, unit: Unit) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("justify".into(), attrs.get("justify").filter(|v| v.is_string()).cloned().unwrap_or(Value::Null));
    for key in DISTANCES {
        fields.insert(key.into(), distance_field(attrs.get(key), unit));
    }
    fields.insert("composer".into(), attrs.get("composer").filter(|v| v.is_string()).cloned().unwrap_or(Value::Null));
    for key in ["roman_hanging", "hyphenate", "bunri_kinshi", "korean_line_break_anywhere"] {
        fields.insert(key.into(), attrs.get(key).filter(|v| v.is_boolean()).cloned().unwrap_or(Value::Null));
    }
    let hyphenation = attrs.get("hyphenation");
    for (field, key, _) in COUNTS {
        fields.insert(field.into(), number_field(hyphenation.and_then(|h| h.get(key))));
    }
    fields.insert("hyphenation_zone".into(), distance_field(hyphenation.and_then(|h| h.get("zone")), unit));
    fields.insert(
        "hyphenate_capitalized".into(),
        hyphenation.and_then(|h| h.get("capitalized")).filter(|v| v.is_boolean()).cloned().unwrap_or(Value::Null),
    );
    let justification = attrs.get("justification");
    for (prefix, key, _) in ROWS {
        for (i, column) in COLUMNS.iter().enumerate() {
            fields.insert(format!("{prefix}_{column}"), number_field(justification.and_then(|j| j.get(key)).and_then(|v| v.get(i))));
        }
    }
    fields.insert("auto_leading".into(), number_field(justification.and_then(|j| j.get("auto_leading"))));
    fields.insert("single_word".into(), justification.and_then(|j| j.get("single_word")).filter(|v| v.is_string()).cloned().unwrap_or(Value::Null));
    fields.insert("tabs".into(), tabs_field(attrs.get("tabs"), unit));
    fields
}

/// A field's caption without its colon, for messages that name it.
fn caption(label: &str) -> &str {
    label.trim_end().trim_end_matches([':', '：']).trim_end()
}

/// The caption of the number or distance field `key`, as the dialog shows it.
fn label_of(key: &str) -> &'static str {
    match key {
        "left_indent" => tl!("Left Indent"),
        "right_indent" => tl!("Right Indent"),
        "first_line_indent" => tl!("First-line Left Indent"),
        "space_before" => tl!("Space Before Paragraph"),
        "space_after" => tl!("Space After Paragraph"),
        "words_longer_than" => tl!("Words Longer Than:"),
        "after_first" => tl!("After First:"),
        "before_last" => tl!("Before Last:"),
        "hyphen_limit" => tl!("Hyphen Limit:"),
        "hyphenation_zone" => tl!("Hyphenation Zone:"),
        "auto_leading" => tl!("Auto Leading:"),
        k if k.starts_with("word") => tl!("Word Spacing:"),
        k if k.starts_with("letter") => tl!("Letter Spacing:"),
        _ => tl!("Glyph Scaling:"),
    }
}

/// The text of field `key`, trimmed; `None` when it is empty or unset.
fn typed<'a>(d: &'a Values, key: &str) -> Option<&'a str> {
    d.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// The distance typed in field `key` (in `unit` unless it names its own), in points: `None` when
/// the field is empty, an error when it is not a distance or is outside `lo..=hi` points.
fn distance(d: &Values, key: &str, unit: Unit, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    let Some(s) = typed(d, key) else { return Ok(None) };
    let label = caption(label_of(key));
    let pt = unit.parse(s).ok_or_else(|| format!("{label} must be a number"))?;
    if !(lo..=hi).contains(&pt) {
        let (min, max) = (unit.format(lo), unit.format(hi));
        return Err(format!("{label} must be a number from {min} to {max}"));
    }
    Ok(Some(pt))
}

/// The number typed in field `key`: `None` when the field is empty, an error when it is not a
/// number or is outside `lo..=hi`.
fn number(d: &Values, key: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    let Some(s) = typed(d, key) else { return Ok(None) };
    let label = caption(label_of(key));
    let n = vectorcraft_doc::parse_number(s.trim_end_matches('%').trim_end()).filter(|n| n.is_finite());
    let n = n.ok_or_else(|| format!("{label} must be a number"))?;
    if !(lo..=hi).contains(&n) {
        let (min, max) = (format_number(lo), format_number(hi));
        return Err(format!("{label} must be a number from {min} to {max}"));
    }
    Ok(Some(n))
}

/// The whole number typed in field `key`: `None` when it is empty, an error when it is not a whole
/// number from `lo` to `hi`.
fn count(d: &Values, key: &str, lo: f64, hi: f64) -> Result<Option<u32>, String> {
    let Some(n) = number(d, key, lo, hi)? else { return Ok(None) };
    if n.fract() != 0.0 {
        let (label, min, max) = (caption(label_of(key)), format_number(lo), format_number(hi));
        return Err(format!("{label} must be a whole number from {min} to {max}"));
    }
    // A whole number within `lo..=hi` (at most 25) is exact as a `u32`.
    Ok(Some(n as u32))
}

/// The style's attributes as the dialog's `fields` set them. A field still as it opened keeps its
/// stored value without a check, so a style read from a file with a value out of range can still
/// be renamed; a tab that was reset writes its unset fields as unset.
pub(crate) fn edited_attrs(fields: &DialogFields, opening: &Opening) -> Result<Map<String, Value>, String> {
    let d = Values::new(fields);
    let original = &opening.attrs;
    let unit = opening.unit;
    let original_fields = canonical_map(fields_from_attrs(original, unit));
    let mut attrs = original.clone();
    let unchanged = |key: &str, tab: &str| !d.reset(tab) && d.get(key) == original_fields.get(key);
    let keep_or = |attrs: &mut Map<String, Value>, key: &str, unchanged: bool, value: Option<Value>| {
        attrs.remove(key);
        let value = if unchanged { original.get(key).cloned() } else { value };
        if let Some(value) = value {
            attrs.insert(key.into(), value);
        }
    };
    let choice = |key: &str| d.get(key).filter(|v| v.is_string()).cloned();
    let switch = |key: &str| d.get(key).filter(|v| v.is_boolean()).cloned();

    let same = unchanged("justify", "basic");
    keep_or(&mut attrs, "justify", same, choice("justify").filter(|v| v.as_str().is_some_and(|v| JUSTIFY_VALUES.contains(&v))));
    for key in DISTANCES {
        let same = unchanged(key, "basic");
        let value = if same { None } else { distance(&d, key, unit, -MAX_DISTANCE, MAX_DISTANCE)?.map(Value::from) };
        keep_or(&mut attrs, key, same, value);
    }

    let same = unchanged("composer", "composition");
    keep_or(
        &mut attrs,
        "composer",
        same,
        choice("composer").filter(|v| matches!(v.as_str(), Some("singleLine" | "everyLine" | "standardSingleLine" | "standardEveryLine"))),
    );
    let same = unchanged("roman_hanging", "composition");
    keep_or(&mut attrs, "roman_hanging", same, switch("roman_hanging"));

    let same = unchanged("hyphenate", "hyphenation");
    keep_or(&mut attrs, "hyphenate", same, switch("hyphenate"));
    let same = HYPHENATION_FIELDS.iter().all(|key| unchanged(key, "hyphenation"));
    let value = if same { None } else { hyphenation(&d, original, &original_fields, unit)? };
    keep_or(&mut attrs, "hyphenation", same, value);

    let same = JUSTIFICATION_FIELDS.iter().all(|key| unchanged(key, "justification"));
    let value = if same { None } else { justification(&d, original, &original_fields)? };
    keep_or(&mut attrs, "justification", same, value);

    let same = unchanged("tabs", "tabs");
    let value = if same { None } else { tab_stops(&d, original, opening)? };
    keep_or(&mut attrs, "tabs", same, value);

    for key in ["bunri_kinshi", "korean_line_break_anywhere"] {
        let same = unchanged(key, "eastAsian");
        keep_or(&mut attrs, key, same, switch(key));
    }
    Ok(attrs)
}

/// The `tabs` attribute the Tabs section sets: none while the list is unset; otherwise its stops
/// sorted by position. Of stops within 1e-6 pt of each other, the one edited last (`__tabEdited`)
/// is kept, else the first in the list. A stop as the style has it is kept without a check, and any
/// other is read by [`read_stop`]. An error when the list has more than 256 stops and more than the
/// style has.
fn tab_stops(d: &Values, original: &Map<String, Value>, opening: &Opening) -> Result<Option<Value>, String> {
    let unit = opening.unit;
    let rows = match d.get("tabs") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(rows)) => rows,
        Some(_) => {
            let key = "tabs";
            return Err(format!("`{key}` must be an array"));
        }
    };
    let kept: Vec<TabStop> =
        original.get("tabs").and_then(Value::as_array).into_iter().flatten().filter_map(|s| serde_json::from_value(s.clone()).ok()).collect();
    let max = TabStop::MAX_STOPS;
    if rows.len() > max.max(kept.len()) {
        return Err(format!("at most {max} tab stops"));
    }
    let label = caption(if opening.vertical { tl!("Y:") } else { tl!("X:") });
    let kept_rows: Vec<Value> = kept.iter().map(|s| stop_field(s, unit)).collect();
    let mut stops = Vec::with_capacity(rows.len());
    for row in rows {
        match kept_rows.iter().position(|k| k == row) {
            Some(i) => stops.push(kept[i].clone()),
            None => stops.push(read_stop(row, unit, label)?),
        }
    }
    let edited = d.dialog.int("__tabEdited").and_then(|i| usize::try_from(i).ok());
    let mut order: Vec<usize> = (0..stops.len()).collect();
    order.sort_by(|&a, &b| stops[a].position.total_cmp(&stops[b].position).then(a.cmp(&b)));
    let mut keep: Vec<usize> = Vec::with_capacity(order.len());
    for i in order {
        match keep.last_mut() {
            Some(last) if (stops[*last].position - stops[i].position).abs() < 1e-6 => {
                if Some(i) == edited {
                    *last = i;
                }
            }
            _ => keep.push(i),
        }
    }
    let stops: Vec<TabStop> = keep.into_iter().map(|i| stops[i].clone()).collect();
    Ok(Some(serde_json::to_value(stops).map_err(|e| e.to_string())?))
}

/// The `hyphenation` attribute the Hyphenation fields set: none when every field is empty or
/// unset; otherwise the style's options (or the defaults) with each changed field written over
/// them, an empty one at its default.
fn hyphenation(d: &Values, original: &Map<String, Value>, original_fields: &Map<String, Value>, unit: Unit) -> Result<Option<Value>, String> {
    let empty = |key: &str| typed(d, key).is_none() && !d.get(key).is_some_and(Value::is_boolean);
    if HYPHENATION_FIELDS.iter().all(|key| empty(key)) {
        return Ok(None);
    }
    let reset = d.reset("hyphenation");
    let mut h: HyphenationOptions =
        if reset { None } else { original.get("hyphenation").and_then(|v| serde_json::from_value(v.clone()).ok()) }.unwrap_or_default();
    let defaults = HyphenationOptions::default();
    let changed = |key: &str| reset || d.get(key) != original_fields.get(key);
    for (field, key, (lo, hi)) in COUNTS {
        if !changed(field) {
            continue;
        }
        let given = count(d, field, lo, hi)?;
        let value = match key {
            "words_longer_than" => &mut h.words_longer_than,
            "after_first" => &mut h.after_first,
            "before_last" => &mut h.before_last,
            _ => &mut h.hyphen_limit,
        };
        let default = match key {
            "words_longer_than" => defaults.words_longer_than,
            "after_first" => defaults.after_first,
            "before_last" => defaults.before_last,
            _ => defaults.hyphen_limit,
        };
        *value = given.unwrap_or(default);
    }
    if changed("hyphenation_zone") {
        h.zone = distance(d, "hyphenation_zone", unit, 0.0, MAX_DISTANCE)?.unwrap_or(defaults.zone);
    }
    if changed("hyphenate_capitalized") {
        h.capitalized = d.get("hyphenate_capitalized").and_then(Value::as_bool).unwrap_or(defaults.capitalized);
    }
    Ok(Some(serde_json::to_value(h).map_err(|e| e.to_string())?))
}

/// The `justification` attribute the Justification fields set, as [`hyphenation`] builds its
/// own; an error when a row with a changed field has a Minimum above its Desired value or a
/// Desired value above its Maximum. A row left as it was keeps the style's values.
fn justification(d: &Values, original: &Map<String, Value>, original_fields: &Map<String, Value>) -> Result<Option<Value>, String> {
    if JUSTIFICATION_FIELDS.iter().all(|key| typed(d, key).is_none()) {
        return Ok(None);
    }
    let reset = d.reset("justification");
    let mut j: JustificationOptions =
        if reset { None } else { original.get("justification").and_then(|v| serde_json::from_value(v.clone()).ok()) }.unwrap_or_default();
    let defaults = JustificationOptions::default();
    let changed = |key: &str| reset || d.get(key) != original_fields.get(key);
    for (prefix, key, (lo, hi)) in ROWS {
        let (values, default) = match key {
            "word_spacing" => (&mut j.word_spacing, defaults.word_spacing),
            "letter_spacing" => (&mut j.letter_spacing, defaults.letter_spacing),
            _ => (&mut j.glyph_scaling, defaults.glyph_scaling),
        };
        let mut row_changed = false;
        for (i, column) in COLUMNS.iter().enumerate() {
            let field = format!("{prefix}_{column}");
            if changed(&field) {
                values[i] = number(d, &field, lo, hi)?.unwrap_or(default[i]);
                row_changed = true;
            }
        }
        if row_changed && !(values[0] <= values[1] && values[1] <= values[2]) {
            return Err(tl!("Minimum must not exceed Desired, nor Desired Maximum.").to_string());
        }
    }
    if changed("auto_leading") {
        let (lo, hi) = JustificationOptions::AUTO_LEADING;
        j.auto_leading = number(d, "auto_leading", lo, hi)?.unwrap_or(defaults.auto_leading);
    }
    if changed("single_word") {
        let given = d.get("single_word").and_then(Value::as_str).filter(|v| SINGLE_WORD_VALUES.contains(v));
        j.single_word = match given {
            Some(v) => serde_json::from_value(json!(v)).map_err(|e| e.to_string())?,
            None => defaults.single_word,
        };
    }
    Ok(Some(serde_json::to_value(j).map_err(|e| e.to_string())?))
}

/// The `tabs` entries of `fields` (none while unset), the selected one's index (`__tabSel`, at most the
/// last) and the index of the one edited last (`__tabEdited`).
fn tab_rows(fields: &DialogFields) -> (Vec<Value>, usize, Option<usize>) {
    let rows: Vec<Value> = fields.get("tabs").map(value_of).as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
    let sel = fields.text("__tabSel").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0).min(rows.len().saturating_sub(1));
    (rows, sel, fields.int("__tabEdited").and_then(|i| usize::try_from(i).ok()))
}

/// Writes the stops and the selected one. The editors are seeded again from the selected stop at the
/// next draw of the Tabs row.
fn put_tabs(fields: &mut DialogFields, rows: Vec<Value>, sel: usize) {
    fields.set("tabs", FieldValue::Json(Value::Array(rows)));
    fields.set("__tabSel", FieldValue::Text(sel.to_string()));
    fields.remove("__tabShown");
}

/// The position (pt) Add gives a new stop among `rows`: half an inch past the furthest stop that
/// reads, half an inch when none does, and at most 100000 pt. `None` while the list holds 256 stops
/// or when a stop already sits at that position.
fn new_tab_position(rows: &[Value], unit: Unit) -> Option<f64> {
    if rows.len() >= TabStop::MAX_STOPS {
        return None;
    }
    let positions: Vec<f64> = rows.iter().filter_map(|r| stop_position(r, unit)).collect();
    let furthest = positions.iter().copied().filter(|pt| (0.0..=TabStop::MAX_POSITION).contains(pt)).reduce(f64::max);
    let position = furthest.map_or(DEFAULT_TAB_INTERVAL, |pt| pt + DEFAULT_TAB_INTERVAL).min(TabStop::MAX_POSITION);
    (!positions.iter().any(|pt| (pt - position).abs() < 1e-6)).then_some(position)
}

/// Add in Tabs: a left stop at [`new_tab_position`], which becomes the selected and last edited stop.
/// Nothing when that gives no position.
pub(crate) fn add_tab(fields: &mut DialogFields, unit: Unit) {
    let (mut rows, _, _) = tab_rows(fields);
    let Some(position) = new_tab_position(&rows, unit) else { return };
    rows.push(stop_field(&TabStop { position, align: TabAlign::Left, leader: String::new(), align_on: '.' }, unit));
    let at = rows.len() - 1;
    put_tabs(fields, rows, at);
    fields.set("__tabEdited", FieldValue::Int(at as i64));
}

/// Delete Tab in Tabs: remove the selected stop; the next one (or the last) becomes selected.
pub(crate) fn delete_tab(fields: &mut DialogFields) {
    let (mut rows, sel, edited) = tab_rows(fields);
    if sel >= rows.len() {
        return;
    }
    rows.remove(sel);
    match edited {
        Some(e) if e == sel => {
            fields.remove("__tabEdited");
        }
        Some(e) if e > sel => fields.set("__tabEdited", FieldValue::Int(e as i64 - 1)),
        _ => {}
    }
    let at = sel.min(rows.len().saturating_sub(1));
    put_tabs(fields, rows, at);
}

/// Clear All Tabs in Tabs: an empty list of stops, which the style then sets.
pub(crate) fn clear_tabs(fields: &mut DialogFields) {
    put_tabs(fields, vec![], 0);
    fields.remove("__tabEdited");
}

/// Copies what the editors show into the selected stop when they were seeded from it.
fn sync_editors(fields: &mut DialogFields) {
    let (mut rows, sel, _) = tab_rows(fields);
    if fields.int("__tabShown") != i64::try_from(sel).ok() {
        return;
    }
    let Some(stop) = rows.get_mut(sel).and_then(Value::as_object_mut) else { return };
    let mut changed = false;
    for (field, key) in TAB_EDITORS {
        let typed = fields.text(field).unwrap_or_default();
        if stop.get(key).and_then(Value::as_str).unwrap_or_default() != typed {
            stop.insert(key.into(), json!(typed));
            changed = true;
        }
    }
    if changed {
        fields.set("tabs", FieldValue::Json(Value::Array(rows)));
        fields.set("__tabEdited", FieldValue::Int(sel as i64));
    }
}

/// Shows the selected stop in the editors, writing only what differs so a settled pass edits nothing.
fn seed_editors(fields: &mut DialogFields) {
    let (rows, sel, _) = tab_rows(fields);
    for (field, key) in TAB_EDITORS {
        let shown = rows.get(sel).and_then(|r| r.get(key)).and_then(Value::as_str).unwrap_or_default();
        if fields.text(field).unwrap_or_default() != shown {
            fields.set(field, FieldValue::Text(shown.into()));
        }
    }
    let selected = sel.to_string();
    if !rows.is_empty() && fields.text("__tabSel") != Some(selected.as_str()) {
        fields.set("__tabSel", FieldValue::Text(selected));
    }
    let shown = i64::try_from(sel).ok().filter(|_| sel < rows.len());
    if fields.int("__tabShown") != shown {
        match shown {
            Some(at) => fields.set("__tabShown", FieldValue::Int(at)),
            None => {
                fields.remove("__tabShown");
            }
        }
    }
}

/// The Add, Delete Tab and Clear All Tabs buttons, which also keep the editors and the stop they edit in step.
fn tabs_buttons(unit: Unit) -> Param {
    Param::custom("tabs", tl!("Tabs"), move |ui, fields, _| {
        sync_editors(fields);
        let (rows, _, _) = tab_rows(fields);
        let can_add = new_tab_position(&rows, unit).is_some();
        let (mut add, mut delete, mut clear) = (false, false, false);
        let line = Row::new().wrap(true).show(ui, |ui, _| {
            add = clicked(&Button::new(tl!("Add")).enabled(can_add).show(ui));
            delete = clicked(&Button::new(tl!("Delete Tab")).enabled(!rows.is_empty()).show(ui));
            clear = clicked(&Button::new(tl!("Clear All Tabs")).enabled(!rows.is_empty()).show(ui));
        });
        if add {
            add_tab(fields, unit);
        } else if delete {
            delete_tab(fields);
        } else if clear {
            clear_tabs(fields);
        }
        seed_editors(fields);
        line.output
    })
    .companions(["__tabSel", "__tabEdited", "__tabShown", "__tabAlign", "__tabPosition", "__tabLeader", "__tabAlignOn"])
}

fn tabs_rows(fields: &DialogFields, o: &Opening) -> Vec<PropertyRow> {
    let aligns: Vec<(String, Label)> =
        TAB_ALIGNS.iter().map(|(a, key)| ((*key).to_string(), crate::panels::tabs::align_name(*a, o.vertical).into())).collect();
    let (stops, _, _) = tab_rows(fields);
    let listed: Vec<(String, Label)> = stops
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let name = r
                .get("align")
                .and_then(Value::as_str)
                .and_then(|key| TAB_ALIGNS.iter().find(|(_, k)| *k == key))
                .map_or("", |(a, _)| crate::panels::tabs::align_name(*a, o.vertical));
            let mut line = format!("{}    {name}", r.get("position").and_then(Value::as_str).unwrap_or_default());
            if let Some(leader) = r.get("leader").and_then(Value::as_str).filter(|l| !l.is_empty()) {
                line += &format!("    {leader}");
            }
            (i.to_string(), line.into())
        })
        .collect();
    rows(vec![
        Param::new("__tabAlign", tl!("Alignment:"), ParamKind::choice(aligns, None)),
        text("__tabPosition", if o.vertical { tl!("Y:") } else { tl!("X:") }),
        text("__tabLeader", tl!("Leader:")),
        text("__tabAlignOn", tl!("Align On:")).enabled_when("__tabAlign", FieldValue::Text("decimal".into())),
        tabs_buttons(o.unit),
        Param::new("__tabSel", tl!("Tab Stops"), ParamKind::choice(listed, None)),
    ])
}

fn basic_rows() -> Vec<PropertyRow> {
    let alignments: Vec<(&'static str, &'static str)> = std::iter::once(("Auto", tl!("Auto")))
        .chain(JUSTIFY_VALUES[1..].iter().copied().zip(crate::panels::paragraph::ALIGNMENTS.iter().map(|(_, _, label, _)| tl!(*label))))
        .collect();
    let mut params = vec![select("justify", tl!("Alignment:"), &alignments)];
    params.extend(DISTANCES.iter().map(|key| text(*key, label_of(key))));
    rows(params)
}

fn composition_rows() -> Vec<PropertyRow> {
    let composers = [
        ("singleLine", tl!("Middle Eastern & South Asian Single-line Composer")),
        ("everyLine", tl!("Middle Eastern & South Asian Every-line Composer")),
        ("standardSingleLine", tl!("Standard Single-line Composer")),
        ("standardEveryLine", tl!("Standard Every-line Composer")),
    ];
    rows(vec![select("composer", tl!("Composition"), &composers), tri_state("roman_hanging", tl!("Roman Hanging Punctuation"))])
}

// PLAN: Hyphenation Zone is gated on Hyphenation alone; the Single-line Composer condition needs a gate on two keys.
fn hyphenation_rows() -> Vec<PropertyRow> {
    let on = FieldValue::Bool(true);
    let mut params = vec![tri_state("hyphenate", tl!("Hyphenation"))];
    params.extend(COUNTS.iter().map(|(key, _, _)| text(*key, label_of(key)).enabled_when("hyphenate", on.clone())));
    params.push(text("hyphenation_zone", label_of("hyphenation_zone")).enabled_when("hyphenate", on.clone()));
    params.push(tri_state("hyphenate_capitalized", tl!("Hyphenate Capitalized Words")).enabled_when("hyphenate", on));
    rows(params)
}

fn justification_rows() -> Vec<PropertyRow> {
    let mut params = vec![];
    for (prefix, _, _) in ROWS {
        let row_label = label_of(prefix);
        for column in COLUMNS {
            params.push(text(format!("{prefix}_{column}"), format!("{} {}", caption(row_label), column_name(column))));
        }
    }
    params.push(text("auto_leading", label_of("auto_leading")));
    let choices: Vec<(&'static str, &'static str)> =
        SINGLE_WORD_VALUES.into_iter().zip([tl!("Full Justify"), tl!("Align left"), tl!("Align center"), tl!("Align right")]).collect();
    params.push(select("single_word", tl!("Single Word Justification:"), &choices));
    rows(params)
}

fn east_asian_rows() -> Vec<PropertyRow> {
    vec![
        PropertyRow::field(tri_state("bunri_kinshi", tl!("Bunri-Kinshi"))),
        PropertyRow::heading(tl!("Korean Composition Settings")),
        PropertyRow::field(tri_state("korean_line_break_anywhere", tl!("Korean Line Break Anywhere"))),
    ]
}

/// The heading of a spacing column, for the accessible name of its field.
fn column_name(column: &str) -> &'static str {
    match column {
        "min" => tl!("Minimum"),
        "desired" => tl!("Desired"),
        _ => tl!("Maximum"),
    }
}

/// The seven tabs of Paragraph Style Options and New Paragraph Style over the dialog's current values.
/// Every tab but General has a Reset Panel.
pub(crate) fn tabs(fields: &DialogFields, opening: &Opening) -> Vec<PropertiesTab> {
    let titles = [
        tl!("General"),
        tl!("Basic Paragraph Formats"),
        tl!("Tabs"),
        tl!("Composition"),
        tl!("Hyphenation"),
        tl!("Justification"),
        tl!("East Asian Composition Settings"),
    ];
    let bodies = [
        general_rows(edited_attrs(fields, opening)),
        basic_rows(),
        tabs_rows(fields, opening),
        composition_rows(),
        hyphenation_rows(),
        justification_rows(),
        east_asian_rows(),
    ];
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
pub(crate) fn hosted(key: &'static str, title: &'static str, opening: &Arc<Opening>, name: &str) -> HostedPropertiesDialog {
    let rows_of = Arc::clone(opening);
    HostedPropertiesDialog::new(key, opening.fields(name), move |values| {
        let now = values.value().cloned().unwrap_or_else(DialogFields::new);
        craft::ui::patterns::form_dialogs::PropertiesDialog::new(key, title, tabs(&now, &rows_of), values)
            .tab_layout(craft::ui::patterns::form_dialogs::TabLayout::List)
    })
    .held_ok()
}

/// The open Paragraph Style Options dialog.
pub(crate) struct ParaStyleOptions {
    inner: HostedPropertiesDialog,
    opening: Arc<Opening>,
}

impl ParaStyleOptions {
    /// The style's attributes as the dialog's values set them, or why they are refused.
    pub(crate) fn edited(&self) -> Result<Map<String, Value>, String> {
        edited_attrs(self.inner.fields(), &self.opening)
    }

    fn command(&self, attrs: Map<String, Value>) -> Value {
        let to = self.inner.fields().text("name").unwrap_or_default();
        json!({"name": self.opening.style, "to": to, "attrs": attrs})
    }
}
style_dialog!(ParaStyleOptions, KIND);

pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    crate::panels::character::end_typing(app);
    let unit = app.session.general_unit();
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let def = doc.para_styles.iter().find(|s| s.name == name).ok_or_else(|| format!("no style `{name}`"))?;
    let opening = Arc::new(Opening::new(name, def.attrs.clone(), unit, vertical_text(app)));
    let dialog = ParaStyleOptions { inner: hosted(KIND, tl!("Paragraph Style Options"), &opening, name).previews(), opening };
    app.dialogs().ok_or("no window")?.with(|host| host.open(dialog));
    Ok(json!({"dialog": KIND}))
}

/// Reset Panel: clears the settings of the tab the open Paragraph Style Options or New Paragraph Style
/// dialog shows.
pub fn reset_panel(app: &mut VectorcraftApp) -> Result<Value, String> {
    let dialogs = app.dialogs().ok_or("no dialog open")?;
    let reset = dialogs.with(|host| {
        let options = host.get_mut::<ParaStyleOptions>().map(|d| (KIND, d.hosted_mut().reset_tab()));
        options.or_else(|| {
            let new = host.get_mut::<super::new_text_style::NewTextStyle>().filter(|d| d.para);
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
    let Some(frame) = dialogs.with(|host| host.get_mut::<ParaStyleOptions>().map(ParaStyleOptions::frame)) else {
        if matches!(dialogs.with(|host| host.take::<ParaStyleOptions>()), Some(DialogResult::Cancel)) {
            std::mem::take(&mut app.para_style_preview).cancel(app);
        }
        return;
    };
    let live = std::mem::take(&mut app.para_style_preview);
    let (live, done) = drive(app, live, tl!("Paragraph Style Options"), "paraStyle.edit", &frame);
    app.para_style_preview = live;
    if let Some(done) = done {
        let answer = done.map(drop).map_err(|why| reason(&why));
        dialogs.with(|host| {
            if let Some(d) = host.get_mut::<ParaStyleOptions>() {
                d.hosted_mut().answer_ok(answer);
            }
        });
    }
}

#[cfg(test)]
mod tests;
