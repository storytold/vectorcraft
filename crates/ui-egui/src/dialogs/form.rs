//! The parameter lists of the dialogs: the generic form dialogs' rows ([`grid_params`]), a plug-in's rows from its
//! manifest ([`plugin_params`]) and the JSON ↔ dialog values pair ([`fields_of`], [`json_of`]) that hosted parameter
//! dialogs use.

use std::collections::HashMap;

use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind, encode};
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use craft::ui::runtime::measure;
use craft::ui::text::TextField;
use craft::ui::value_drag::SliderScale;
use serde_json::{Map, Value, json};
use vectorcraft_plugins::{Manifest, Param as PluginParam, ParamSpec};

/// The fields of the form dialogs ([`grid_params`]) that are distances, by dialog kind.
fn lengths(kind: &str) -> &'static [&'static str] {
    match kind {
        "move" => &["dx", "dy", "distance"],
        "rectangle" | "ellipse" | "rectangularGrid" | "polarGrid" => &["width", "height"],
        "roundedRectangle" => &["width", "height", "radius"],
        "polygon" | "spiral" => &["radius"],
        "star" => &["radius1", "radius2"],
        "lineSegment" => &["length"],
        "arc" => &["lengthX", "lengthY"],
        "offsetPath" => &["offset"],
        _ => &[],
    }
}

/// The fields of the form dialogs ([`grid_params`]) that are angles or percentages, with their unit: fields that
/// ↑/↓ step by one (Shift: ten) and that do math, as the distances do.
fn unit_of(kind: &str, key: &str) -> Option<UnitKind> {
    match (kind, key) {
        ("rotate" | "shear" | "reflect" | "move" | "lineSegment", "angle") | ("shear", "axisAngle") => Some(UnitKind::Degrees),
        ("scale", "sx" | "sy") | ("spiral", "decay") => Some(UnitKind::Percent),
        _ => None,
    }
}

/// The fields of the form dialogs ([`grid_params`]) that the dialog's own body draws, by dialog kind.
fn own_rows(kind: &str) -> &'static [&'static str] {
    match kind {
        // The grids' Reference Point is their 9-point locator.
        "rectangularGrid" | "polarGrid" => &["reference"],
        _ => &[],
    }
}

/// The fields of the form dialogs ([`grid_params`]) that come first, in this order (the others follow by
/// name).
fn order(kind: &str) -> &'static [&'static str] {
    match kind {
        "move" => &["dx", "dy", "distance", "angle"],
        "offsetPath" => &["offset", "joins", "miterLimit"],
        "rectangularGrid" => &["width", "height", "rows", "rowSkew", "columns", "columnSkew"],
        "polarGrid" => &["width", "height", "concentric", "concentricSkew", "radial", "radialSkew"],
        "spiral" => &["radius", "decay", "segments"],
        "artboardOptions" => &["name", "width", "height"],
        "lineSegment" => &["length", "angle"],
        "shear" => &["angle", "axis", "axisAngle"],
        _ => &[],
    }
}

/// The form dialog ([`grid_params`]) fields that are a grid tool's divider skew, a slider in percent: the
/// words at the two ends of its rail (where the dividers crowd to), in the UI language.
fn skew_ends(kind: &str, key: &str) -> Option<(&'static str, &'static str)> {
    match (kind, key) {
        ("rectangularGrid", "rowSkew") => Some((tl!("Bottom"), tl!("Top"))),
        // The radial dividers gather on that side of the top one.
        ("rectangularGrid", "columnSkew") | ("polarGrid", "radialSkew") => Some((tl!("Left"), tl!("Right"))),
        ("polarGrid", "concentricSkew") => Some((tl!("Inside"), tl!("Outside"))),
        _ => None,
    }
}

/// The choices of a form dialog's ([`grid_params`]) dropdown field, as (value, label in the UI
/// language); empty for a field typed as text.
fn choices(kind: &str, key: &str) -> Vec<(&'static str, &'static str)> {
    // The axes in the "axis" context: some languages read the plain Horizontal and Vertical as
    // type orientations.
    let axis = |value, label| (value, crate::i18n::tr_ctx(crate::i18n::current(), "axis", label));
    match (kind, key) {
        ("offsetPath", "joins") => vec![("miter", tl!("Miter")), ("round", tl!("Round")), ("bevel", tl!("Bevel"))],
        ("average", "axis") => vec![axis("horizontal", "Horizontal"), axis("vertical", "Vertical"), axis("both", "Both")],
        ("reflect" | "shear", "axis") => vec![axis("horizontal", "Horizontal"), axis("vertical", "Vertical"), axis("angle", "Angle")],
        _ => vec![],
    }
}

/// A row's label from its text: the parameter rows draw the colon themselves.
fn label(text: &str) -> Label {
    Label::from(text.trim_end_matches([':', '：', ' ', '\u{a0}']).to_owned())
}

/// The generic form dialog's rows ([`order`]ed; positions, indices, UI-only keys, booleans and the kind's
/// [`own_rows`] left out): the distances in `units`, the [`choices`] as dropdowns and the [`skew_ends`] fields as
/// sliders. A dialog adds its own rows after the returned list.
pub(crate) fn grid_params(kind: &str, fields: &Map<String, Value>, units: LengthUnits) -> Vec<Param> {
    let (distances, first, own) = (lengths(kind), order(kind), own_rows(kind));
    let mut rows: Vec<(&String, &Value)> = fields
        .iter()
        .filter(|(k, v)| !matches!(k.as_str(), "x" | "y" | "origin" | "index") && !k.starts_with("__") && !v.is_boolean())
        .filter(|(k, _)| !own.contains(&k.as_str()))
        .collect();
    // Stable: the fields `order` doesn't name keep their order after the ones it does.
    rows.sort_by_key(|(k, _)| first.iter().position(|f| *f == k.as_str()).unwrap_or(first.len()));
    rows.into_iter()
        .map(|(k, value)| {
            let key = k.as_str();
            let number = value.as_f64().unwrap_or(0.0);
            let name = match (kind, key) {
                ("shear", "axisAngle") => tl!("Axis Angle").to_owned(),
                ("rectangularGrid", "rows") => tl!("Horizontal Dividers:").to_owned(),
                ("rectangularGrid", "columns") => tl!("Vertical Dividers:").to_owned(),
                ("polarGrid", "concentric") => tl!("Concentric Dividers:").to_owned(),
                ("polarGrid", "radial") => tl!("Radial Dividers:").to_owned(),
                _ => humanized(key),
            };
            let options = choices(kind, key);
            if distances.contains(&key) {
                Param::number(key, label(&name), number, 1.0, 3).length(units)
            } else if let Some(unit) = unit_of(kind, key) {
                let row = Param::number(key, label(&name), number, 1.0, 3).unit(unit);
                // Shear's Axis Angle is the axis while the Angle axis is chosen, and keeps its value otherwise.
                if (kind, key) == ("shear", "axisAngle") { row.enabled_when("axis", "angle") } else { row }
            } else if let Some((low, high)) = skew_ends(kind, key) {
                let max = vectorcraft_geom::shapes::GRID_SKEW_MAX;
                Param::new(key, label(tl!("Skew:")), ParamKind::range(-max, max, number, SliderScale::Linear))
                    .unit(UnitKind::Percent)
                    .marks_labelled([(-max, low.to_owned()), (max, high.to_owned())])
            } else if !options.is_empty() {
                let entries = options.iter().map(|(v, l)| ((*v).to_owned(), Label::from((*l).to_owned()))).collect();
                Param::new(key, label(&name), ParamKind::choice(entries, value.as_str().map(str::to_owned)))
            } else if let Some(count) = value.as_i64() {
                // PLAN: the sheet types every other field as text; a numeric value stays a number so the command
                // reads the integer it was given.
                Param::new(key, label(&name), ParamKind::int(None, None, count))
            } else if value.is_number() {
                Param::number(key, label(&name), number, 1.0, 3)
            } else {
                Param::new(key, label(&name), ParamKind::text(value.as_str().unwrap_or_default()))
            }
        })
        .collect()
}

/// A colour as a JSON value holds it: `#rrggbb` is display sRGB as typed, a colour object is a colour of the working
/// RGB space ([`vectorcraft_plugins::manifest::color_value`]).
fn color_in(value: &Value) -> Option<[f32; 3]> {
    let color = match value {
        Value::String(hex) => vectorcraft_color::Color::from_hex(hex)?,
        _ => vectorcraft_plugins::manifest::color_value(value)?,
    };
    let [r, g, b, _] = color.to_rgba8(1.0);
    Some([r, g, b].map(|v| f32::from(v) / 255.0))
}

/// The dialog value of the JSON `value` of the row `row` (`None` for a key no row has).
fn field_value(row: Option<&Param>, value: &Value) -> Option<FieldValue> {
    match row.map(|p| &p.kind) {
        Some(ParamKind::Range { .. } | ParamKind::Number { .. } | ParamKind::Angle { .. } | ParamKind::OptionalNumber { .. }) => {
            value.as_f64().map(FieldValue::Number)
        }
        Some(ParamKind::Color { .. }) => color_in(value).map(FieldValue::Color),
        Some(ParamKind::Point { .. }) => match value.as_array()?.as_slice() {
            [x, y] => Some(FieldValue::Point([x.as_f64()?, y.as_f64()?])),
            _ => None,
        },
        // A pair of radio buttons over a flag (Relative / Absolute): the ids are "true" and "false".
        Some(ParamKind::Radio { .. }) if value.is_boolean() => value.as_bool().map(|b| FieldValue::Text(b.to_string())),
        // Use the renderer's mode for each spelling, including Normal for unknown names and the effect's default for
        // a legacy value that is not text.
        Some(ParamKind::Blend { default, .. }) => value
            .as_str()
            .map(|s| vectorcraft_effects::blend_mode_id(vectorcraft_effects::blend_mode(s)))
            .or_else(|| default.clone())
            .map(FieldValue::Text),
        // A custom row's list, and one a key no row has carries through (an export's `artboards: [0, 1]`), stay JSON: a
        // list of whole numbers would read back as a grid, and one of two or three as a point or colour.
        Some(ParamKind::Custom(_)) | None if value.is_array() => Some(FieldValue::Json(value.clone())),
        _ => serde_json::from_value(value.clone()).ok(),
    }
}

/// The JSON value of the dialog value `value` of the row `row`: a number is encoded as [`typed_number`] does.
fn json_value(row: Option<&Param>, value: &FieldValue) -> Value {
    match value {
        FieldValue::Number(x) => typed_number(*x),
        FieldValue::Color([r, g, b]) => json!(format!("#{:02x}{:02x}{:02x}", encode(*r), encode(*g), encode(*b))),
        FieldValue::Text(id) if matches!(id.as_str(), "true" | "false") && row.is_some_and(|p| matches!(p.kind, ParamKind::Radio { .. })) => {
            json!(id == "true")
        }
        FieldValue::Json(json) => json.clone(),
        other => serde_json::to_value(other).unwrap_or_default(),
    }
}

/// The dialog values of the JSON `json`, each by its row's kind in `params`; a key no row has passes through.
pub(crate) fn fields_of(params: &[Param], json: &Map<String, Value>) -> DialogFields {
    let mut fields = DialogFields::new();
    for (key, value) in json {
        if let Some(v) = field_value(params.iter().find(|p| p.key == *key), value) {
            fields.set(key.as_str(), v);
        }
    }
    fields
}

/// Shows `body` in a `Stack` as rows of a custom dialog row that edit `fields`. The stack may run its body more than
/// once in a pass: every run edits a copy of the values as they stand here, and the copy of the run that is not a
/// measuring run replaces them after the stack.
pub(super) fn stacked(ui: &mut egui::Ui, fields: &mut DialogFields, mut body: impl FnMut(&mut egui::Ui, &mut DialogFields)) -> egui::Response {
    let before = fields.clone();
    let mut edited = None;
    let shown = Stack::new().show(ui, |ui, _| {
        let mut run = before.clone();
        body(ui, &mut run);
        if measure::effect(ui.ctx(), "dialog rows edit their values") {
            edited = Some(run);
        }
    });
    if let Some(edited) = edited {
        *fields = edited;
    }
    shown.output
}

/// The command or effect parameters of the dialog values `fields`: the UI-only keys (`__*`, `preview`) are dropped.
pub(crate) fn json_of(params: &[Param], fields: &DialogFields) -> Map<String, Value> {
    fields
        .iter()
        .filter(|(k, _)| !k.starts_with("__") && *k != "preview")
        .map(|(k, v)| (k.to_owned(), json_value(params.iter().find(|p| p.key == k), v)))
        .collect()
}

/// The (label, value) choices of a parameter that picks one of some values.
pub(super) type Choices = &'static [(&'static str, &'static str)];

/// The label of one of an effect dialog's choices in the UI language, in the "effect" context: some
/// languages read the plain Soft, Horizontal and Vertical (Grain's types) as type settings.
pub(super) fn choice_label(label: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "effect", label)
}

/// A number typed in a command's dialog as JSON: a whole number as an integer, as counts are read
/// (`as_u64`: Object Mosaic's columns and rows, #1000), which `12.0` isn't; a fraction stays one.
/// Commands that take any number read either.
pub(crate) fn typed_number(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 1e15 { json!(x as i64) } else { json!(x) }
}

/// The unit a plug-in names, as the framework spells it; `None` for a unit it has no kind for.
fn unit_kind(unit: &str) -> Option<UnitKind> {
    match unit {
        "px" => Some(UnitKind::Pixels),
        "%" => Some(UnitKind::Percent),
        "°" => Some(UnitKind::Degrees),
        "pt" => Some(UnitKind::Points),
        "in" => Some(UnitKind::Inches),
        "cm" => Some(UnitKind::Centimetres),
        "mm" => Some(UnitKind::Millimetres),
        _ => None,
    }
}

/// A row of a text field whose text `read` turns into the row's value (`None`: the text is not a value and the row
/// keeps what it had), shown by `shown`; for the values no framework row types: a number list, a whole number past
/// what a number field holds exactly, a text with a length limit.
fn text_row(
    key: &str,
    name: Label,
    limit: Option<usize>,
    start: FieldValue,
    shown: fn(&FieldValue) -> String,
    read: impl Fn(&str) -> Option<FieldValue> + Send + Sync + 'static,
) -> Param {
    let (owned, field_name) = (key.to_owned(), name.clone());
    Param::custom(key, name, move |ui, fields, _| {
        let text = fields.get(&owned).map(shown).unwrap_or_default();
        let out = TextField::new(field_name.clone(), Bind::given(text)).limit(limit).show(ui);
        if let Some(value) = out.value().and_then(|typed| read(typed)) {
            fields.set(owned.as_str(), value);
        }
        out.response
    })
    .opens_with(start)
}

/// The row of the plug-in parameter `key`, without its visibility.
fn plugin_row(key: &str, p: &PluginParam) -> Param {
    let unit_text = p.ui.unit.as_deref().unwrap_or_default();
    let unit = unit_kind(unit_text);
    let base = p.ui.label.clone().unwrap_or_else(|| humanize(key));
    let base = base.trim_end_matches(':');
    // A unit the framework has no kind for stays in the label, so the plug-in's own unit is still shown.
    let name = Label::from(if unit.is_none() && !unit_text.is_empty() { format!("{base} ({unit_text})") } else { base.to_owned() });
    let mut row = match &p.spec {
        ParamSpec::Number { min, max, default } => Param::new(key, name, ParamKind::range(*min, *max, *default, SliderScale::Linear)),
        ParamSpec::Int { min, max, default } => {
            // An f64 holds every whole number up to 2^53, so the number field (arrows, wheel, arithmetic) is exact
            // there. Past it, the number typed is read as an integer.
            const EXACT: i64 = 1 << 53;
            let start = (*default).max(*min).min(*max);
            if -EXACT <= *min && *max <= EXACT {
                Param::new(key, name, ParamKind::int(Some(*min), Some(*max), start))
            } else {
                let (min, max) = (*min, *max);
                text_row(
                    key,
                    name,
                    None,
                    FieldValue::Int(start),
                    |v| match v {
                        FieldValue::Int(i) => i.to_string(),
                        FieldValue::Json(j) => j.as_i64().map(|i| i.to_string()).unwrap_or_default(),
                        _ => String::new(),
                    },
                    move |text| text.trim().parse::<i64>().ok().map(|x| FieldValue::Int(x.max(min).min(max))),
                )
            }
        }
        ParamSpec::Bool { default } => Param::new(key, name, ParamKind::bool(*default)),
        ParamSpec::Choice { options, default } => {
            let entries = options.iter().map(|o| (o.clone(), Label::from(o.clone()))).collect();
            Param::new(key, name, ParamKind::choice(entries, default.clone()))
        }
        ParamSpec::Text { default, max_length } => text_row(
            key,
            name,
            Some(*max_length),
            FieldValue::Text(default.clone()),
            |v| if let FieldValue::Text(s) = v { s.clone() } else { String::new() },
            |text| Some(FieldValue::Text(text.to_owned())),
        ),
        ParamSpec::NumberList { .. } => {
            let (spec, start) = (p.spec.clone(), p.spec.default_value());
            text_row(
                key,
                name,
                None,
                FieldValue::Json(start),
                |v| if let FieldValue::Json(list) = v { number_list_text(list) } else { String::new() },
                move |text| read_number_list(text, &spec).map(FieldValue::Json),
            )
        }
        ParamSpec::Color { default } => Param::new(key, name, ParamKind::color(color_in(default))),
    };
    if let Some(unit) = unit {
        row = row.unit(unit);
    }
    if let Some(help) = &p.ui.help {
        row = row.tooltip(help.clone());
    }
    if let Some(url) = &p.ui.help_url {
        row = row.help_link(url.clone());
    }
    row
}

/// A plug-in's parameter rows from its manifest (the filter, live effect and exporter dialogs): a row each, labelled
/// as the manifest says (else from the parameter's name), with its help on the label and on a help button that opens
/// its page ([`open_help`]). Parameters outside any group come first, then each group as a section that folds
/// ([`Manifest::sections`]).
///
/// A parameter whose `visibleWhen` the values don't meet is hidden by its row, as [`Manifest::visible`] decides it:
/// shown only when every parameter it names is itself shown, so a row carries its own conditions and those of the
/// parameters they name. `visibleWhen` names only earlier parameters, so one pass in declaration order has each
/// one's conditions ready.
// PLAN: the values are not read here (the rows hide themselves from the dialog's values); the contract's signature
// keeps the parameter.
pub(crate) fn plugin_params(manifest: &Manifest, _fields: &Map<String, Value>) -> Vec<Param> {
    let mut conditions: HashMap<&str, Vec<(String, Vec<FieldValue>)>> = HashMap::new();
    for (name, p) in &manifest.params {
        let mut own: Vec<(String, Vec<FieldValue>)> =
            p.ui.visible_when
                .iter()
                .map(|(other, allowed)| (other.clone(), allowed.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()))
                .collect();
        for other in p.ui.visible_when.keys() {
            own.extend(conditions.get(other.as_str()).cloned().unwrap_or_default());
        }
        conditions.insert(name.as_str(), own);
    }
    let mut rows = Vec::new();
    for (group, members) in manifest.sections() {
        if let Some(group) = group {
            rows.push(Param::group(Label::from(group.to_owned())));
        }
        for (key, p) in members {
            let when = conditions.remove(key.as_str()).unwrap_or_default();
            rows.push(when.into_iter().fold(plugin_row(key, p), |row, (other, allowed)| row.visible_when_in(other, allowed)));
        }
    }
    rows
}

/// Opens the help page `link` of a plug-in parameter (`ParamRowsOutput::help_link`). The browser is another app: a
/// click automation sent opens nothing.
pub(crate) fn open_help(app: &mut crate::VectorcraftApp, link: &str) {
    if app.refuse_automation("the plug-in help button").is_ok() {
        app.open_url(link);
    }
}

/// A `numberList` value as its field shows it: "45, 135".
pub(crate) fn number_list_text(cur: &Value) -> String {
    let numbers: Vec<String> = cur.as_array().into_iter().flatten().filter_map(Value::as_f64).map(number_text).collect();
    numbers.join(", ")
}

/// The list typed in a `numberList` parameter's field: numbers separated by commas, semicolons or
/// spaces, each clamped into the parameter's range. `None` (the value stays as it was) when a
/// number doesn't read, there are too few or too many, or `spec` isn't a number list.
pub(crate) fn read_number_list(text: &str, spec: &ParamSpec) -> Option<Value> {
    let ParamSpec::NumberList { min, max, min_items, max_items, .. } = spec else { return None };
    let parts = text.split([',', ';', ' ']).filter(|p| !p.trim().is_empty());
    let list: Vec<f64> = parts.map(|p| vectorcraft_doc::parse_number(p).filter(|x| x.is_finite())).collect::<Option<_>>()?;
    (*min_items..=*max_items).contains(&list.len()).then(|| json!(list.into_iter().map(|x| x.clamp(*min, *max)).collect::<Vec<f64>>()))
}

/// A number of a list as its field shows it: at most 4 decimals, without trailing zeros.
fn number_text(x: f64) -> String {
    let s = format!("{x:.4}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// A field label from its camelCase key (`miterLimit` → "Miter Limit:").
pub(crate) fn humanize(k: &str) -> String {
    let mut s = String::new();
    for (i, c) in k.chars().enumerate() {
        if i == 0 {
            s.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            s.push(' ');
            s.push(c);
        } else {
            s.push(c);
        }
    }
    match s.as_str() {
        "Ppi" => "Resolution:".into(),
        "Dx" => "Horizontal".into(),
        "Dy" => "Vertical".into(),
        "Sx" => "Horizontal %".into(),
        "Sy" => "Vertical %".into(),
        "Radius1" => "Radius 1".into(),
        "Radius2" => "Radius 2".into(),
        "Dash1" => "Dash 1:".into(),
        "Dash2" => "Dash 2:".into(),
        "Dash3" => "Dash 3:".into(),
        "Dash4" => "Dash 4:".into(),
        "Dash5" => "Dash 5:".into(),
        "Gap1" => "Gap 1:".into(),
        "Gap2" => "Gap 2:".into(),
        "Gap3" => "Gap 3:".into(),
        "Gap4" => "Gap 4:".into(),
        "Gap5" => "Gap 5:".into(),
        "Align Corners" => "Align to Corners and Ends:".into(),
        "Max Radius" => "Max. Radius:".into(),
        "Blur Mode" => "Mode:".into(),
        "Channel1" => "Channel 1:".into(),
        "Channel2" => "Channel 2:".into(),
        "Channel3" => "Channel 3:".into(),
        "Channel4" => "Channel 4:".into(),
        "Light Dark Balance" => "Light/Dark Balance:".into(),
        "Include Cmy Blacks" => "Include Blacks with CMY:".into(),
        "Align To Path" => "Align to Path:".into(),
        "Overlap Variation" | "Curviness Variation" | "Spacing Variation" => "Variation:".into(),
        "Create" => "Create New Fields by:".into(),
        "Height Width" => "Height/Width:".into(),
        "Traps With Process Color" => "Traps with Process Color:".into(),
        "Cols" => "Columns:".into(),
        "Flip Cols" => "Flip Columns:".into(),
        "Each" => "One per Object:".into(),
        _ => format!("{s}:"),
    }
}

/// A style attribute as the style dialogs list it: the [`humanized`] key, then its value.
pub(super) fn attribute_line(key: &str, value: &Value) -> String {
    format!("{} {value}", humanized(key))
}

/// [`humanize`] translated where it is painted: the ":" stays outside the translated text and is
/// written as the UI language writes a label's colon ([`crate::i18n::colon`]).
pub(super) fn humanized(k: &str) -> String {
    let h = humanize(k);
    match h.strip_suffix(':') {
        Some(base) => crate::i18n::colon(tl!(base)),
        None => tl!(&h).to_string(),
    }
}

#[cfg(test)]
mod plugin_field_tests {
    use super::*;
    use crate::i18n::LangExt as _;

    /// [`json_of`] reads back what [`fields_of`] took in: numbers as the command reads them, a flag pair as a bool,
    /// unknown keys through, UI-only keys dropped.
    #[test]
    fn fields_and_json_round_trip_by_row_kind() {
        let flag = ParamKind::radio(vec![("true".into(), Label::from("Relative")), ("false".into(), Label::from("Absolute"))], None);
        let params = [Param::new("size", "Size", ParamKind::range(0.0, 10.0, 1.0, SliderScale::Linear)), Param::new("relative", "Mode", flag)];
        let Value::Object(json) = json!({"size": 3, "relative": false, "other": "x", "__effect": "e", "preview": true}) else { return };
        assert_eq!(Value::Object(json_of(&params, &fields_of(&params, &json))), json!({"size": 3, "relative": false, "other": "x"}));
    }

    /// The `{n} ppi` row keeps the number first and a space before the unit, so every reader of the row (the
    /// resolution fields' units) can show the unit after the number.
    #[test]
    fn every_catalog_writes_the_ppi_unit_after_the_number() {
        for lang in crate::i18n::Lang::all() {
            let row = crate::i18n::tr(lang, "{n} ppi");
            let unit = row.strip_prefix("{n}").unwrap_or_default();
            assert!(unit.starts_with(char::is_whitespace) && !unit.trim().is_empty(), "{}: {row:?}", lang.code());
        }
    }

    #[test]
    fn number_lists_read_as_typed_within_their_limits() {
        let spec = ParamSpec::NumberList { min: 0.0, max: 360.0, default: vec![45.0], min_items: 1, max_items: 3 };
        assert_eq!(number_list_text(&json!([45.0, 135.5, 0.125])), "45, 135.5, 0.125");
        assert_eq!(read_number_list("10; 400 20", &spec), Some(json!([10.0, 360.0, 20.0])), "separators, and clamped");
        assert_eq!(read_number_list(" 90 , 2*45 ", &spec), Some(json!([90.0, 90.0])));
        for bad in ["", "1, 2, 3, 4", "1, x", "1, inf"] {
            assert_eq!(read_number_list(bad, &spec), None, "{bad:?}");
        }
        assert_eq!(read_number_list("1", &ParamSpec::Bool { default: false }), None, "not a list");
    }
}

#[cfg(test)]
mod typed_number_tests {
    use super::*;

    #[test]
    fn whole_numbers_are_integers_and_fractions_stay_fractions() {
        // Whatever the field held before: a count typed after a fraction is a count again.
        assert_eq!(typed_number(24.0).as_u64(), Some(24));
        assert_eq!(typed_number(-4.0), json!(-4));
        assert_eq!(typed_number(0.0), json!(0));
        assert_eq!(typed_number(2.5), json!(2.5));
        // Numbers past exact integers stay floats.
        assert!(typed_number(1e20).is_f64());
    }
}
