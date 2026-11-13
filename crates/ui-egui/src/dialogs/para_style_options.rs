//! Paragraph Style Options fields shared by editing and creating a paragraph style: General, Basic
//! Paragraph Formats (alignment, indents and spacing), Tabs (the style's tab stops), Composition
//! (composer and Roman Hanging Punctuation), Hyphenation, Justification and East Asian Composition
//! Settings (Bunri-Kinshi and Korean Line Break Anywhere).
//!
//! Each field is unset (empty, or a checkbox in its unset state) or set, and an unset field leaves
//! its attribute out of the style. Distances are shown and typed in the General unit and kept in
//! points. The Hyphenation and Justification options are one attribute each (`hyphenation`,
//! `justification`): when any of their fields is set, a field left empty takes its default value.
//! The `tabs` field is unset (null) or a list of stops, an empty list included, and is the `tabs`
//! attribute. A field still as it opened keeps the style's stored value as it is.
//!
//! Behavior reference (public documentation):
//! <https://helpx.adobe.com/illustrator/using/character-paragraph-styles.html>.

use serde_json::{Map, Value, json};
use vectorcraft_doc::text::DEFAULT_TAB_INTERVAL;
use vectorcraft_doc::{HyphenationOptions, JustificationOptions, TabAlign, TabStop, Unit};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub const KIND: &str = "paraStyleOptions";

/// The section ids, in the order the section list shows them.
pub(crate) const SECTIONS: [&str; 7] = ["general", "basic", "tabs", "composition", "hyphenation", "justification", "eastAsian"];
/// The alignments of a tab stop with the names the `tabs` field gives them, in the Tabs panel's
/// order.
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

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Paragraph Style Options").into(),
    body,
    confirm,
    preview: true,
    inputs: |_| {
        &[
            "name",
            "justify",
            "left_indent",
            "right_indent",
            "first_line_indent",
            "space_before",
            "space_after",
            "composer",
            "roman_hanging",
            "hyphenate",
            "words_longer_than",
            "after_first",
            "before_last",
            "hyphen_limit",
            "hyphenation_zone",
            "hyphenate_capitalized",
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
            "bunri_kinshi",
            "korean_line_break_anywhere",
            "tabs",
            "section",
        ]
    },
    min_width: 720.0,
    max_width: Some(720.0),
    ..DialogSpec::FORM
};

pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    crate::panels::character::end_typing(app);
    let unit = app.session.general_unit();
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let def = doc.para_styles.iter().find(|s| s.name == name).ok_or_else(|| format!("no style `{name}`"))?;
    let mut fields = form_fields(&def.attrs, unit);
    fields.insert("__vertical".into(), json!(vertical_text(app)));
    fields.insert("name".into(), json!(name));
    fields.insert("__style".into(), json!(name));
    fields.insert("section".into(), json!("general"));
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({"dialog": KIND}))
}

/// The dialog fields for a style's `attrs` in `unit`, with the attributes and the unit they come
/// from, which [`edited_attrs`] reads back.
pub(crate) fn form_fields(attrs: &Map<String, Value>, unit: Unit) -> Map<String, Value> {
    let mut fields = fields_from_attrs(attrs, unit);
    fields.insert("__attrs".into(), Value::Object(attrs.clone()));
    fields.insert("__unit".into(), json!(unit.key()));
    fields
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
fn typed<'a>(d: &'a Dialog, key: &str) -> Option<&'a str> {
    d.fields.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// The distance typed in field `key` (in `unit` unless it names its own), in points: `None` when
/// the field is empty, an error when it is not a distance or is outside `lo..=hi` points.
fn distance(d: &Dialog, key: &str, unit: Unit, lo: f64, hi: f64) -> Result<Option<f64>, String> {
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
fn number(d: &Dialog, key: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
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
fn count(d: &Dialog, key: &str, lo: f64, hi: f64) -> Result<Option<u32>, String> {
    let Some(n) = number(d, key, lo, hi)? else { return Ok(None) };
    if n.fract() != 0.0 {
        let (label, min, max) = (caption(label_of(key)), format_number(lo), format_number(hi));
        return Err(format!("{label} must be a whole number from {min} to {max}"));
    }
    // A whole number within `lo..=hi` (at most 25) is exact as a `u32`.
    Ok(Some(n as u32))
}

/// The unit the dialog shows distances in.
fn unit_of(d: &Dialog) -> Unit {
    d.fields.get("__unit").and_then(Value::as_str).and_then(Unit::named_east_asian).unwrap_or_default()
}

/// The style's attributes as the dialog's fields set them. A field still as it opened keeps its
/// stored value without a check, so a style read from a file with a value out of range can still
/// be renamed.
pub(crate) fn edited_attrs(d: &Dialog) -> Result<Map<String, Value>, String> {
    let original = d.fields.get("__attrs").and_then(Value::as_object).cloned().unwrap_or_default();
    let unit = unit_of(d);
    let original_fields = fields_from_attrs(&original, unit);
    let mut attrs = original.clone();
    let unchanged = |key: &str, reset: &str| !d.bool(reset) && d.fields.get(key) == original_fields.get(key);
    let keep_or = |attrs: &mut Map<String, Value>, key: &str, unchanged: bool, value: Option<Value>| {
        attrs.remove(key);
        let value = if unchanged { original.get(key).cloned() } else { value };
        if let Some(value) = value {
            attrs.insert(key.into(), value);
        }
    };
    let choice = |key: &str| d.fields.get(key).filter(|v| v.is_string()).cloned();
    let switch = |key: &str| d.fields.get(key).filter(|v| v.is_boolean()).cloned();

    let same = unchanged("justify", "__resetBasic");
    keep_or(&mut attrs, "justify", same, choice("justify").filter(|v| v.as_str().is_some_and(|v| JUSTIFY_VALUES.contains(&v))));
    for key in DISTANCES {
        let same = unchanged(key, "__resetBasic");
        let value = if same { None } else { distance(d, key, unit, -MAX_DISTANCE, MAX_DISTANCE)?.map(Value::from) };
        keep_or(&mut attrs, key, same, value);
    }

    let same = unchanged("composer", "__resetComposition");
    keep_or(&mut attrs, "composer", same, choice("composer").filter(|v| matches!(v.as_str(), Some("singleLine" | "everyLine"))));
    let same = unchanged("roman_hanging", "__resetComposition");
    keep_or(&mut attrs, "roman_hanging", same, switch("roman_hanging"));

    let same = unchanged("hyphenate", "__resetHyphenation");
    keep_or(&mut attrs, "hyphenate", same, switch("hyphenate"));
    let same = HYPHENATION_FIELDS.iter().all(|key| unchanged(key, "__resetHyphenation"));
    let value = if same { None } else { hyphenation(d, &original, &original_fields, unit)? };
    keep_or(&mut attrs, "hyphenation", same, value);

    let same = JUSTIFICATION_FIELDS.iter().all(|key| unchanged(key, "__resetJustification"));
    let value = if same { None } else { justification(d, &original, &original_fields)? };
    keep_or(&mut attrs, "justification", same, value);

    let same = unchanged("tabs", "__resetTabs");
    let value = if same { None } else { tab_stops(d, &original, unit)? };
    keep_or(&mut attrs, "tabs", same, value);

    for key in ["bunri_kinshi", "korean_line_break_anywhere"] {
        let same = unchanged(key, "__resetEastAsian");
        keep_or(&mut attrs, key, same, switch(key));
    }
    Ok(attrs)
}

/// The `tabs` attribute the Tabs section sets: none while the list is unset; otherwise its stops
/// sorted by position. Of stops within 1e-6 pt of each other, the one edited last (`__tabEdited`)
/// is kept, else the first in the list. A stop as the style has it is kept without a check, and any
/// other is read by [`read_stop`]. An error when the list has more than 256 stops and more than the
/// style has.
fn tab_stops(d: &Dialog, original: &Map<String, Value>, unit: Unit) -> Result<Option<Value>, String> {
    let rows = match d.fields.get("tabs") {
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
    let label = caption(if d.bool("__vertical") { tl!("Y:") } else { tl!("X:") });
    let kept_rows: Vec<Value> = kept.iter().map(|s| stop_field(s, unit)).collect();
    let mut stops = Vec::with_capacity(rows.len());
    for row in rows {
        match kept_rows.iter().position(|k| k == row) {
            Some(i) => stops.push(kept[i].clone()),
            None => stops.push(read_stop(row, unit, label)?),
        }
    }
    let edited = d.fields.get("__tabEdited").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
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
fn hyphenation(d: &Dialog, original: &Map<String, Value>, original_fields: &Map<String, Value>, unit: Unit) -> Result<Option<Value>, String> {
    let empty = |key: &str| typed(d, key).is_none() && !d.fields.get(key).is_some_and(Value::is_boolean);
    if HYPHENATION_FIELDS.iter().all(|key| empty(key)) {
        return Ok(None);
    }
    let reset = d.bool("__resetHyphenation");
    let mut h: HyphenationOptions =
        if reset { None } else { original.get("hyphenation").and_then(|v| serde_json::from_value(v.clone()).ok()) }.unwrap_or_default();
    let defaults = HyphenationOptions::default();
    let changed = |key: &str| reset || d.fields.get(key) != original_fields.get(key);
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
        h.capitalized = d.fields.get("hyphenate_capitalized").and_then(Value::as_bool).unwrap_or(defaults.capitalized);
    }
    Ok(Some(serde_json::to_value(h).map_err(|e| e.to_string())?))
}

/// The `justification` attribute the Justification fields set, as [`hyphenation`] builds its
/// own; an error when a row with a changed field has a Minimum above its Desired value or a
/// Desired value above its Maximum. A row left as it was keeps the style's values.
fn justification(d: &Dialog, original: &Map<String, Value>, original_fields: &Map<String, Value>) -> Result<Option<Value>, String> {
    if JUSTIFICATION_FIELDS.iter().all(|key| typed(d, key).is_none()) {
        return Ok(None);
    }
    let reset = d.bool("__resetJustification");
    let mut j: JustificationOptions =
        if reset { None } else { original.get("justification").and_then(|v| serde_json::from_value(v.clone()).ok()) }.unwrap_or_default();
    let defaults = JustificationOptions::default();
    let changed = |key: &str| reset || d.fields.get(key) != original_fields.get(key);
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
        let given = d.fields.get("single_word").and_then(Value::as_str).filter(|v| SINGLE_WORD_VALUES.contains(v));
        j.single_word = match given {
            Some(v) => serde_json::from_value(json!(v)).map_err(|e| e.to_string())?,
            None => defaults.single_word,
        };
    }
    Ok(Some(serde_json::to_value(j).map_err(|e| e.to_string())?))
}

/// Reset Panel in Basic Paragraph Formats: Alignment and the five indent and spacing fields become
/// unset.
fn reset_basic(d: &mut Dialog) {
    d.fields.insert("__resetBasic".into(), json!(true));
    d.fields.insert("justify".into(), Value::Null);
    for key in DISTANCES {
        d.fields.insert(key.into(), json!(""));
    }
}

/// Reset Panel in Composition: the composer and Roman Hanging Punctuation become unset.
fn reset_composition(d: &mut Dialog) {
    d.fields.insert("__resetComposition".into(), json!(true));
    d.fields.insert("composer".into(), Value::Null);
    d.fields.insert("roman_hanging".into(), Value::Null);
}

/// Reset Panel in Hyphenation: Hyphenation and its options become unset.
fn reset_hyphenation(d: &mut Dialog) {
    d.fields.insert("__resetHyphenation".into(), json!(true));
    d.fields.insert("hyphenate".into(), Value::Null);
    d.fields.insert("hyphenate_capitalized".into(), Value::Null);
    for key in HYPHENATION_FIELDS.iter().filter(|k| **k != "hyphenate_capitalized") {
        d.fields.insert((*key).into(), json!(""));
    }
}

/// Reset Panel in Justification: the Justification options become unset.
fn reset_justification(d: &mut Dialog) {
    d.fields.insert("__resetJustification".into(), json!(true));
    d.fields.insert("single_word".into(), Value::Null);
    for key in JUSTIFICATION_FIELDS.iter().filter(|k| **k != "single_word") {
        d.fields.insert((*key).into(), json!(""));
    }
}

/// Reset Panel in Tabs: the tab stops become unset, which leaves `tabs` out of the style.
fn reset_tabs(d: &mut Dialog) {
    d.fields.insert("__resetTabs".into(), json!(true));
    d.fields.insert("tabs".into(), Value::Null);
    d.fields.remove("__tabEdited");
    d.fields.insert("__tabSel".into(), json!(0));
}

/// Reset Panel in East Asian Composition Settings: Bunri-Kinshi and Korean Line Break Anywhere
/// become unset.
fn reset_east_asian(d: &mut Dialog) {
    d.fields.insert("__resetEastAsian".into(), json!(true));
    d.fields.insert("bunri_kinshi".into(), Value::Null);
    d.fields.insert("korean_line_break_anywhere".into(), Value::Null);
}

/// Whether the section `d` shows has a Reset Panel: every section but General.
pub(crate) fn can_reset(d: &Dialog) -> bool {
    ["basic", "tabs", "composition", "hyphenation", "justification", "eastAsian"].contains(&d.str("section").as_str())
}

/// Reset Panel: clear the settings of the section `d` shows.
pub(crate) fn reset_section(d: &mut Dialog) {
    match d.str("section").as_str() {
        "basic" => reset_basic(d),
        "tabs" => reset_tabs(d),
        "composition" => reset_composition(d),
        "hyphenation" => reset_hyphenation(d),
        "justification" => reset_justification(d),
        "eastAsian" => reset_east_asian(d),
        _ => {}
    }
}

/// Whether `d` is Paragraph Style Options or New Paragraph Style.
pub(crate) fn is_paragraph_dialog(d: &Dialog) -> bool {
    d.kind == KIND || (d.kind == super::new_text_style::KIND && d.bool("__para"))
}

pub fn reset_panel(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = app.ui.dialog.as_mut().ok_or("no dialog open")?;
    if !is_paragraph_dialog(d) || !can_reset(d) {
        return Err("the open dialog has no Reset".into());
    }
    reset_section(d);
    Ok(json!({"dialog": d.kind}))
}

/// The section list of Paragraph Style Options and New Paragraph Style.
pub(crate) fn section_list(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.vertical(|ui| {
        let labels = [
            tl!("General"),
            tl!("Basic Paragraph Formats"),
            tl!("Tabs"),
            tl!("Composition"),
            tl!("Hyphenation"),
            tl!("Justification"),
            tl!("East Asian Composition Settings"),
        ];
        for (id, label) in SECTIONS.into_iter().zip(labels) {
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

/// A menu whose first entry leaves field `key` unset.
fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[(&str, &str)], enabled: bool) {
    let selected = d.fields.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let shown = options.iter().find(|(value, _)| *value == selected).map_or("", |(_, label)| *label);
    let labels: Vec<&str> = std::iter::once(" ").chain(options.iter().map(|(_, label)| *label)).collect();
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(i) = crate::widgets::dropdown(ui, (KIND, key), shown, &labels, 190.0) {
            d.fields.insert(key.into(), if i == 0 { Value::Null } else { options.get(i - 1).map_or(Value::Null, |(value, _)| json!(value)) });
        }
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

/// A text field in its own grid row, with what follows the number, enabled or not.
fn field_row(ui: &mut egui::Ui, d: &mut Dialog, key: &str, caption: &str, suffix: &str, enabled: bool) {
    let row = ui.label(caption);
    if d.fields.get(key).is_none_or(Value::is_null) {
        d.fields.insert(key.into(), json!(""));
    }
    ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label_inputs_by(row.id, |ui| form::text(ui, d, key, 160.0));
            if !suffix.is_empty() {
                ui.label(suffix);
            }
        });
    });
    ui.end_row();
}

/// Basic Paragraph Formats: Alignment, the left, right and first-line indents, and the space
/// before and after the paragraph.
pub(crate) fn basic_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("paragraph-style-basic").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        let alignments: Vec<(&str, &str)> = std::iter::once(("Auto", tl!("Auto")))
            .chain(JUSTIFY_VALUES[1..].iter().copied().zip(crate::panels::paragraph::ALIGNMENTS.iter().map(|(_, _, label, _)| tl!(*label))))
            .collect();
        let row = ui.label(tl!("Alignment:"));
        ui.label_inputs_by(row.id, |ui| choice(ui, d, "justify", &alignments, true));
        ui.end_row();
        for key in DISTANCES {
            field_row(ui, d, key, label_of(key), "", true);
        }
    });
}

/// The editing fields of the selected tab stop, with the `tabs` entry key each one shows.
const TAB_EDITORS: [(&str, &str); 3] = [("__tabPosition", "position"), ("__tabLeader", "leader"), ("__tabAlignOn", "alignOn")];

/// Tabs: the selected stop's Alignment, X (Y in vertical type), Leader and Align On, which only a
/// decimal stop uses; Add ([`add_tab`]), Delete Tab ([`delete_tab`]) and Clear All Tabs
/// ([`clear_tabs`]); and the list of stops, one row each, a click on which selects its stop, which
/// scrolls when it is taller than 160 points. A change to a field makes its stop the one edited last.
pub(crate) fn tabs_body(ui: &mut egui::Ui, d: &mut Dialog) {
    let vertical = d.bool("__vertical");
    let (mut rows, mut sel, _) = tab_rows(d);
    let mut changed = false;
    let has = sel < rows.len();
    let shown = TAB_EDITORS.map(|(_, key)| rows.get(sel).and_then(|r| r.get(key)).and_then(Value::as_str).unwrap_or("").to_string());
    for ((field, _), text) in TAB_EDITORS.iter().zip(&shown) {
        d.fields.insert((*field).into(), json!(text));
    }
    let align = rows.get(sel).and_then(|r| r.get("align")).and_then(Value::as_str).unwrap_or("left").to_string();
    egui::Grid::new("paragraph-style-tabs").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        let row = ui.label(tl!("Alignment:"));
        let names: Vec<&str> = TAB_ALIGNS.iter().map(|(a, _)| crate::panels::tabs::align_name(*a, vertical)).collect();
        let shown = TAB_ALIGNS.iter().position(|(_, key)| *key == align).and_then(|i| names.get(i).copied()).unwrap_or("");
        ui.add_enabled_ui(has, |ui| {
            ui.label_inputs_by(row.id, |ui| {
                if let Some(i) = crate::widgets::dropdown(ui, (KIND, "tab_align"), shown, &names, 190.0)
                    && let (Some(r), Some((_, key))) = (rows.get_mut(sel).and_then(Value::as_object_mut), TAB_ALIGNS.get(i))
                {
                    r.insert("align".into(), json!(key));
                    changed = true;
                }
            });
        });
        ui.end_row();
        field_row(ui, d, "__tabPosition", if vertical { tl!("Y:") } else { tl!("X:") }, "", has);
        field_row(ui, d, "__tabLeader", tl!("Leader:"), "", has);
        field_row(ui, d, "__tabAlignOn", tl!("Align On:"), "", has && align == "decimal");
    });
    for ((field, key), was) in TAB_EDITORS.iter().zip(&shown) {
        let typed = d.str(field);
        if let Some(r) = rows.get_mut(sel).and_then(Value::as_object_mut)
            && typed != *was
        {
            r.insert((*key).into(), json!(typed));
            changed = true;
        }
    }
    if changed {
        d.fields.insert("tabs".into(), Value::Array(rows.clone()));
        d.fields.insert("__tabEdited".into(), json!(sel));
    }
    ui.add_space(6.0);
    let (mut add, mut delete, mut clear) = (false, false, false);
    let can_add = new_tab_position(&rows, unit_of(d)).is_some();
    ui.horizontal(|ui| {
        add = ui.add_enabled(can_add, egui::Button::new(tl!("Add"))).clicked();
        delete = ui.add_enabled(has, egui::Button::new(tl!("Delete Tab"))).clicked();
        clear = ui.add_enabled(!rows.is_empty(), egui::Button::new(tl!("Clear All Tabs"))).clicked();
    });
    ui.add_space(4.0);
    let reveal = d.fields.remove("__tabScroll").is_some();
    egui::ScrollArea::vertical().id_salt("paragraph-style-tab-list").max_height(160.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, r) in rows.iter().enumerate() {
            let a = r.get("align").and_then(Value::as_str).and_then(|key| TAB_ALIGNS.iter().find(|(_, k)| *k == key)).map(|(a, _)| *a);
            let name = a.map_or("", |a| crate::panels::tabs::align_name(a, vertical));
            let mut text = format!("{}    {name}", r.get("position").and_then(Value::as_str).unwrap_or(""));
            if let Some(leader) = r.get("leader").and_then(Value::as_str).filter(|l| !l.is_empty()) {
                text += &format!("    {leader}");
            }
            let row = ui.selectable_label(i == sel, text);
            if reveal && i == sel {
                ui.scroll_to_rect_animation(row.rect, None, egui::style::ScrollAnimation::none());
            }
            if row.clicked() {
                sel = i;
            }
        }
    });
    d.fields.insert("__tabSel".into(), json!(sel));
    if add {
        add_tab(d);
    } else if delete {
        delete_tab(d);
    } else if clear {
        clear_tabs(d);
    }
}

/// The `tabs` entries of `d` (none while unset), the selected one's index (`__tabSel`, at most the
/// last) and the index of the one edited last (`__tabEdited`).
fn tab_rows(d: &Dialog) -> (Vec<Value>, usize, Option<usize>) {
    let rows: Vec<Value> = d.fields.get("tabs").and_then(Value::as_array).cloned().unwrap_or_default();
    let index = |key: &str| d.fields.get(key).and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
    let sel = index("__tabSel").unwrap_or(0).min(rows.len().saturating_sub(1));
    (rows, sel, index("__tabEdited"))
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

/// Add in Tabs: a left stop at [`new_tab_position`], which becomes the selected and last edited
/// stop and is scrolled into view in the list. Nothing when that gives no position.
pub(crate) fn add_tab(d: &mut Dialog) {
    let unit = unit_of(d);
    let (mut rows, _, _) = tab_rows(d);
    let Some(position) = new_tab_position(&rows, unit) else { return };
    rows.push(stop_field(&TabStop { position, align: TabAlign::Left, leader: String::new(), align_on: '.' }, unit));
    let at = rows.len() - 1;
    d.fields.insert("tabs".into(), Value::Array(rows));
    d.fields.insert("__tabSel".into(), json!(at));
    d.fields.insert("__tabEdited".into(), json!(at));
    d.fields.insert("__tabScroll".into(), json!(true));
}

/// Delete Tab in Tabs: remove the selected stop; the next one (or the last) becomes selected.
pub(crate) fn delete_tab(d: &mut Dialog) {
    let (mut rows, sel, edited) = tab_rows(d);
    if sel >= rows.len() {
        return;
    }
    rows.remove(sel);
    match edited {
        Some(e) if e == sel => {
            d.fields.remove("__tabEdited");
        }
        Some(e) if e > sel => {
            d.fields.insert("__tabEdited".into(), json!(e - 1));
        }
        _ => {}
    }
    d.fields.insert("__tabSel".into(), json!(sel.min(rows.len().saturating_sub(1))));
    d.fields.insert("tabs".into(), Value::Array(rows));
    d.fields.insert("__tabScroll".into(), json!(true));
}

/// Clear All Tabs in Tabs: an empty list of stops, which the style then sets.
pub(crate) fn clear_tabs(d: &mut Dialog) {
    d.fields.insert("tabs".into(), json!([]));
    d.fields.insert("__tabSel".into(), json!(0));
    d.fields.remove("__tabEdited");
}

/// Composition: the Single-line or Every-line Composer, and Roman Hanging Punctuation.
pub(crate) fn composition_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("paragraph-style-composition").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        let row = ui.label(tl!("Composition"));
        let composers = [("singleLine", tl!("Single-line Composer")), ("everyLine", tl!("Every-line Composer"))];
        ui.label_inputs_by(row.id, |ui| choice(ui, d, "composer", &composers, true));
        ui.end_row();
        switch(ui, d, "roman_hanging", tl!("Roman Hanging Punctuation"), true);
    });
}

/// Hyphenation: the Hyphenation checkbox and its options, which are inactive while it is off or
/// unset; Hyphenation Zone is also inactive with the Every-line Composer, which does not use it.
pub(crate) fn hyphenation_body(ui: &mut egui::Ui, d: &mut Dialog) {
    let on = d.fields.get("hyphenate").and_then(Value::as_bool) == Some(true);
    let single_line = d.fields.get("composer").and_then(Value::as_str) == Some("singleLine");
    egui::Grid::new("paragraph-style-hyphenation").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        switch(ui, d, "hyphenate", tl!("Hyphenation"), true);
        for (key, suffix) in [
            ("words_longer_than", tl!("letters")),
            ("after_first", tl!("letters")),
            ("before_last", tl!("letters")),
            ("hyphen_limit", tl!("hyphens")),
        ] {
            field_row(ui, d, key, label_of(key), suffix, on);
        }
        field_row(ui, d, "hyphenation_zone", label_of("hyphenation_zone"), "", on && single_line);
        switch(ui, d, "hyphenate_capitalized", tl!("Hyphenate Capitalized Words"), on);
    });
}

/// Justification: Word Spacing, Letter Spacing and Glyph Scaling (Minimum, Desired and Maximum),
/// Auto Leading and Single Word Justification.
pub(crate) fn justification_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("paragraph-style-justification").num_columns(4).spacing([10.0, 8.0]).show(ui, |ui| {
        ui.label("");
        for heading in [tl!("Minimum"), tl!("Desired"), tl!("Maximum")] {
            ui.label(heading);
        }
        ui.end_row();
        for (prefix, _, _) in ROWS {
            let row_label = label_of(prefix);
            ui.label(row_label);
            for column in COLUMNS {
                let key = format!("{prefix}_{column}");
                if d.fields.get(&key).is_none_or(Value::is_null) {
                    d.fields.insert(key.clone(), json!(""));
                }
                ui.horizontal(|ui| {
                    let name = format!("{} {}", caption(row_label), column_name(column));
                    form::text_named(ui, d, &key, &name, 60.0);
                    ui.label("%");
                });
            }
            ui.end_row();
        }
    });
    ui.add_space(8.0);
    egui::Grid::new("paragraph-style-justification-more").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        field_row(ui, d, "auto_leading", label_of("auto_leading"), "%", true);
        let row = ui.label(tl!("Single Word Justification:"));
        let choices: Vec<(&str, &str)> =
            SINGLE_WORD_VALUES.into_iter().zip([tl!("Full Justify"), tl!("Align left"), tl!("Align center"), tl!("Align right")]).collect();
        ui.label_inputs_by(row.id, |ui| choice(ui, d, "single_word", &choices, true));
        ui.end_row();
    });
}

/// East Asian Composition Settings: Bunri-Kinshi, and Korean Composition Settings › Korean Line
/// Break Anywhere.
pub(crate) fn east_asian_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("paragraph-style-bunri-kinshi").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        switch(ui, d, "bunri_kinshi", tl!("Bunri-Kinshi"), true);
    });
    ui.strong(tl!("Korean Composition Settings"));
    egui::Grid::new("paragraph-style-east-asian").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        switch(ui, d, "korean_line_break_anywhere", tl!("Korean Line Break Anywhere"), true);
    });
}

/// The heading of a spacing column, for the accessible name of its field.
fn column_name(column: &str) -> &'static str {
    match column {
        "min" => tl!("Minimum"),
        "desired" => tl!("Desired"),
        _ => tl!("Maximum"),
    }
}

/// The General section: the style's name and every attribute it sets.
pub(crate) fn general_body(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        let row = ui.label(tl!("Style Name:"));
        ui.label_inputs_by(row.id, |ui| form::text(ui, d, "name", 190.0));
    });
    ui.separator();
    if let Ok(attrs) = edited_attrs(d) {
        for (key, value) in attrs {
            ui.label(format!("{}: {}", super::humanize(&key), value));
        }
    }
}

/// The content of the section `d` shows.
pub(crate) fn section_body(ui: &mut egui::Ui, d: &mut Dialog) {
    match d.str("section").as_str() {
        "basic" => basic_body(ui, d),
        "tabs" => tabs_body(ui, d),
        "composition" => composition_body(ui, d),
        "hyphenation" => hyphenation_body(ui, d),
        "justification" => justification_body(ui, d),
        "eastAsian" => east_asian_body(ui, d),
        _ => general_body(ui, d),
    }
}

/// The key under which the dialog keeps the height its section content was drawn at, for the next
/// frame.
fn content_id(ui: &egui::Ui) -> egui::Id {
    ui.layer_id().id.with("para-style-content")
}

/// The dialog's body: the section list beside the selected section's content, and below them
/// Reset Panel, the Preview checkbox and an invalid value's error. The content is at most 420 points
/// tall, and scrolls when it is taller than that or than the screen less the rest of the window
/// (as drawn last frame). The section list stays put while the content scrolls. Returns whether
/// the dialog asks to be canceled (never: Cancel comes from the button row in `dialogs::show`).
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let window = ui.ctx().memory(|m| m.area_rect(ui.layer_id().id)).map_or(0.0, |r| r.height());
    let content = ui.ctx().data(|m| m.get_temp::<f32>(content_id(ui))).unwrap_or(0.0);
    let rest = (window - content).max(0.0);
    let room = (ui.ctx().content_rect().height() - rest - 2.0 * super::EDGE_GAP).clamp(0.0, 420.0);
    let drawn = ui.horizontal(|ui| {
        section_list(ui, d);
        ui.separator();
        let section = d.str("section");
        let content_w = ui.available_width();
        let shown = egui::ScrollArea::both()
            .id_salt(("para-style-sections", section))
            .max_height(room)
            .min_scrolled_height(room)
            .max_width(content_w)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.vertical(|ui| section_body(ui, d));
            });
        // `inner_rect` is the height offered before a short section shrinks the area to its content.
        shown.inner_rect.height().min(shown.content_size.y)
    });
    let content_h = drawn.inner;
    let was = ui.ctx().data(|m| m.get_temp::<f32>(content_id(ui)));
    ui.ctx().data_mut(|m| m.insert_temp(content_id(ui), content_h));
    // A new height lays the frame out again before it is shown, so the window is placed at its new
    // size and OK and Cancel stay on the screen in the frame of a section switch.
    if was.is_none_or(|was| (was - content_h).abs() > 0.5) {
        ui.ctx().request_discard("paragraph style content height changed");
    }
    reset_button(ui, d);
    match edited_attrs(d) {
        Ok(attrs) => {
            let params = json!({"name": d.str("__style"), "to": d.str("name"), "attrs": attrs});
            form::preview(app, ui, d, tl!("Paragraph Style Options"), "paraStyle.edit", params);
        }
        Err(why) => super::char_style_options::attrs_error(ui, &why),
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let attrs = edited_attrs(d)?;
    form::commit_preview(app, "paraStyle.edit", json!({"name": d.str("__style"), "to": d.str("name"), "attrs": attrs}))
}

#[cfg(test)]
mod tests;
