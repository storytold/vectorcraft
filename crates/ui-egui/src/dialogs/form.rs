//! Field widgets shared by the dialogs: labelled text fields, checkboxes, the generic field grid
//! and the typed parameter editor of the command and effect dialogs.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_plugins::{Manifest, Param, ParamSpec};

use crate::state::Dialog;
use crate::theme::Tokens;

/// Width of a dialog's value fields ([`text`] and [`length`], frame included).
pub(super) const FIELD_W: f32 = 132.0;

/// A labelled text field bound to `d.fields[key]` (one grid row).
pub(super) fn field(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    field_w(ui, d, key, label, FIELD_W - 12.0);
}

/// [`field`] `width` points wide.
pub(super) fn field_w(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, width: f32) {
    let row = row_label(ui, label);
    // The value widget must be named by the real Label already drawn beside it (egui 0.37's
    // kittest fails an unnamed input).
    ui.label_inputs_by(row.id, |ui| text(ui, d, key, width));
    ui.end_row();
}

/// The label cell of a grid row (its `Response`, so the value can be named by it).
pub(super) fn row_label(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(label)).color(t.text_dim))
}

/// A labelled [`length`] field (one grid row).
pub(super) fn length_field(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, unit: Unit) {
    let row = row_label(ui, label);
    ui.label_inputs_by(row.id, |ui| {
        length(ui, d, key, unit, FIELD_W);
    });
    ui.end_row();
}

/// A distance field `width` wide bound to `d.fields[key]`: shown and typed in `unit` (a typed unit,
/// "5 mm", wins), kept in points (a string with a unit, "12 pt", also reads). Returns true when it
/// changed.
pub(super) fn length(ui: &mut egui::Ui, d: &mut Dialog, key: &str, unit: Unit, width: f32) -> bool {
    length_within(ui, d, key, unit, width, None)
}

/// [`length`] whose value is kept within `range` (points): a value typed outside it becomes the
/// nearer end, so the field shows the distance the command applies (#4637).
pub(super) fn length_within(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    key: &str,
    unit: Unit,
    width: f32,
    range: Option<std::ops::RangeInclusive<f64>>,
) -> bool {
    let value = d.fields.contains_key(key).then(|| d.f64(key, 0.0));
    let bounds = range.unwrap_or(crate::widgets::ANY);
    let Some(v) = crate::widgets::num_field_within(ui, ("dlg-len", key), value, unit, width, bounds) else { return false };
    d.fields.insert(key.into(), json!(v));
    true
}

/// What [`length`] shows for `d.fields[key]`, as the screen-reader value of a rail over `range`
/// (in points) beside it: the number in `unit` and the field's text. `None` while the field is
/// blank.
pub(super) fn length_value(d: &Dialog, key: &str, unit: Unit, range: std::ops::RangeInclusive<f64>) -> Option<crate::widgets::SliderValue> {
    let pt = d.fields.contains_key(key).then(|| d.f64(key, 0.0))?;
    let number = unit.number(pt).parse().unwrap_or(unit.from_pt(pt));
    Some(crate::widgets::SliderValue::text(number, unit.from_pt(*range.start())..=unit.from_pt(*range.end()), unit.format(pt)))
}

/// The fields of the form dialogs ([`grid`]) that are distances, by dialog kind.
fn lengths(kind: &str) -> &'static [&'static str] {
    match kind {
        "move" => &["dx", "dy", "distance"],
        "rectangle" | "ellipse" | "rectangularGrid" | "polarGrid" | "artboardOptions" => &["width", "height"],
        "roundedRectangle" => &["width", "height", "radius"],
        "polygon" | "spiral" => &["radius"],
        "star" => &["radius1", "radius2"],
        "lineSegment" => &["length"],
        "arc" => &["lengthX", "lengthY"],
        "offsetPath" => &["offset"],
        _ => &[],
    }
}

/// The fields of the form dialogs ([`grid`]) that are angles or percentages, with what follows the
/// number: fields that ↑/↓ step by one (Shift: ten) and that do math, as the distances do.
fn suffix(kind: &str, key: &str) -> Option<&'static str> {
    match (kind, key) {
        ("rotate" | "shear" | "reflect" | "move" | "lineSegment", "angle") => Some("°"),
        ("scale", "sx" | "sy") | ("spiral", "decay") => Some("%"),
        _ => None,
    }
}

/// The fields of the form dialogs ([`grid`]) that the dialog's own body draws, by dialog kind.
fn own_rows(kind: &str) -> &'static [&'static str] {
    match kind {
        "artboardOptions" => &["color", super::artboard_options::PIXEL_ASPECT],
        // The grids' Reference Point is their 9-point locator.
        "rectangularGrid" | "polarGrid" => &["reference"],
        _ => &[],
    }
}

/// The fields of the form dialogs ([`grid`]) that come first, in this order (the others follow by
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
        _ => &[],
    }
}

/// The form dialog ([`grid`]) fields that are a grid tool's divider skew, a slider in percent: the
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

/// The choices of a form dialog's ([`grid`]) dropdown field, as (value, label in the UI
/// language); empty for a field typed as text.
fn choices(kind: &str, key: &str) -> Vec<(&'static str, &'static str)> {
    // The axes in the "axis" context: some languages read the plain Horizontal and Vertical as
    // type orientations.
    let axis = |value, label| (value, crate::i18n::tr_ctx(crate::i18n::current(), "axis", label));
    match (kind, key) {
        ("offsetPath", "joins") => vec![("miter", tl!("Miter")), ("round", tl!("Round")), ("bevel", tl!("Bevel"))],
        ("average", "axis") => vec![axis("horizontal", "Horizontal"), axis("vertical", "Vertical"), axis("both", "Both")],
        ("reflect", "axis") => vec![axis("horizontal", "Horizontal"), axis("vertical", "Vertical"), axis("angle", "Angle")],
        ("shear", "axis") => vec![axis("horizontal", "Horizontal"), axis("vertical", "Vertical")],
        _ => vec![],
    }
}

/// A text field `width` wide bound to `d.fields[key]`. Returns true when it changed.
pub(super) fn text(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) -> bool {
    text_edit(ui, d, key, width).changed()
}

/// [`text`] for a row whose visible meaning is not the immediately preceding label widget
/// (Export for Screens' Range field follows its radio buttons): screen readers name the field
/// `name`, already translated. The pending label is still consumed, so it cannot name a later
/// field.
pub(super) fn text_named(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, width: f32) -> bool {
    text_edit_named(ui, d, key, name, width).changed()
}

/// [`text`] returning the field's response (focus, Enter).
pub(super) fn text_edit(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) -> egui::Response {
    text_edit_limited(ui, d, key, width, usize::MAX)
}

/// [`text_edit`] with an explicit translated name (see [`text_named`]).
pub(super) fn text_edit_named(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: &str, width: f32) -> egui::Response {
    text_edit_limited_name(ui, d, key, width, usize::MAX, Some(name))
}

/// [`text_edit`] that takes at most `max_chars` characters: typing and pasting stop at the limit,
/// and a longer value already in the field is kept, with nothing added to it until it has fewer
/// than `max_chars` characters.
pub(super) fn text_edit_limited(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32, max_chars: usize) -> egui::Response {
    text_edit_limited_name(ui, d, key, width, max_chars, None)
}

/// The text field shared by [`text_edit_limited`] and [`text_edit_named`]: `name` wins over the
/// adjacent label, and either way the pending label is taken so it can't leak to a later field.
fn text_edit_limited_name(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32, max_chars: usize, name: Option<&str>) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let label = crate::widgets::take_previous_label(ui);
    let mut s = d.str(key);
    let id = ui.scope_id().with(("dlg-text", key));
    crate::widgets::take_dialog_focus(ui, id, &s);
    let r = egui::Frame::NONE
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            crate::theme::strong_selected_text(ui);
            ui.add(
                egui::TextEdit::singleline(&mut s)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .align(egui::Align2::LEFT_CENTER)
                    .desired_width(width)
                    .char_limit(max_chars),
            )
        });
    if r.inner.changed() {
        d.fields.insert(key.into(), Value::String(s));
    }
    match name {
        Some(name) => r.inner.accessible_name(name),
        None => crate::widgets::label_field(r.inner, label),
    }
}

/// A text box `rows` lines tall and `width` wide bound to `d.fields[key]`. Enter adds a line (it
/// doesn't press OK while the box has focus).
pub(super) fn text_area(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32, rows: usize) -> egui::Response {
    text_area_named(ui, d, key, None, width, rows)
}

/// [`text_area`] with an optional explicit translated name; the pending adjacent label is consumed
/// either way.
pub(super) fn text_area_named(ui: &mut egui::Ui, d: &mut Dialog, key: &str, name: Option<&str>, width: f32, rows: usize) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let label = crate::widgets::take_previous_label(ui);
    let mut s = d.str(key);
    let r = egui::Frame::NONE
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| {
            crate::theme::strong_selected_text(ui);
            ui.add(egui::TextEdit::multiline(&mut s).frame(egui::Frame::NONE).desired_width(width).desired_rows(rows))
        });
    if r.inner.changed() {
        d.fields.insert(key.into(), Value::String(s));
    }
    if r.inner.has_focus() {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
    match name {
        Some(name) => r.inner.accessible_name(name),
        None => crate::widgets::label_field(r.inner, label),
    }
}

/// A checkbox bound to `d.fields[key]` (its box shows while unchecked too).
pub(super) fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let b = d.bool(key);
    if crate::widgets::check(ui, label, b, true) {
        d.fields.insert(key.into(), Value::Bool(!b));
    }
}

/// A dropdown row bound to the string `d.fields[key]`: `label` in a column `widths.0` wide, the
/// dropdown `widths.1` wide; `options` are (value, label).
pub(super) fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, widths: (f32, f32), options: &[(&str, &str)]) {
    crate::widgets::label_row(ui, label, widths.0, |ui| dropdown(ui, d, key, widths.1, options, false));
}

/// A dropdown `width` wide bound to the string `d.fields[key]`; `options` are (value, label), the
/// labels shown as they are when `translated` (else the dropdown translates them). A value that
/// isn't among them shows as it is.
fn dropdown(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32, options: &[(&str, &str)], translated: bool) {
    let cur = d.str(key);
    let shown = options.iter().find(|(v, _)| v.eq_ignore_ascii_case(&cur)).map_or(cur.as_str(), |(_, l)| l);
    let labels: Vec<&str> = options.iter().map(|(_, l)| *l).collect();
    let chosen = if translated {
        crate::widgets::dropdown_names(ui, key, shown, &labels, width)
    } else {
        crate::widgets::dropdown(ui, key, shown, &labels, width)
    };
    if let Some((value, _)) = chosen.and_then(|i| options.get(i)) {
        d.fields.insert(key.into(), json!(value));
    }
}

/// The generic dialog body: a field per non-boolean value (positions, indices, UI-only keys and the
/// kind's [`own_rows`] hidden) in the kind's [`order`], the distances in `unit`, the [`choices`] as
/// dropdowns and the [`skew_ends`] fields as sliders.
pub(super) fn grid(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    grid_then(ui, d, unit, |_, _| {});
}

/// [`grid`], then the rows `more` draws in the same grid, so that their labels and fields line up
/// with the generic ones.
pub(super) fn grid_then(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit, more: impl FnOnce(&mut egui::Ui, &mut Dialog)) {
    let rail = Tokens::get(ui.ctx()).input_border;
    let (lengths, first, own) = (lengths(&d.kind), order(&d.kind), own_rows(&d.kind));
    let mut keys: Vec<String> = d
        .fields
        .iter()
        .filter(|(k, v)| !matches!(k.as_str(), "x" | "y" | "origin" | "index") && !k.starts_with("__") && !v.is_boolean())
        .filter(|(k, _)| !own.contains(&k.as_str()))
        .map(|(k, _)| k.clone())
        .collect();
    // Stable: the fields `order` doesn't name keep their order after the ones it does.
    keys.sort_by_key(|k| first.iter().position(|f| f == k).unwrap_or(first.len()));
    egui::Grid::new("dlg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        for k in keys {
            let options = choices(&d.kind, &k);
            if lengths.contains(&k.as_str()) {
                length_field(ui, d, &k, &humanized(&k), unit);
            } else if let Some(suffix) = suffix(&d.kind, &k) {
                let row = row_label(ui, &humanized(&k));
                ui.label_inputs_by(row.id, |ui| {
                    if let Some(v) = crate::widgets::plain_field(ui, ("dlg-num", &k), d.f64(&k, 0.0), suffix, 3, FIELD_W) {
                        d.fields.insert(k, json!(v));
                    }
                });
                ui.end_row();
            } else if let Some(ends) = skew_ends(&d.kind, &k) {
                let max = vectorcraft_geom::shapes::GRID_SKEW_MAX;
                row_label(ui, tl!("Skew:"));
                ui.vertical(|ui| {
                    ui.horizontal(|ui| slider_field_named(ui, d, &k, tl!("Skew:"), -max..=max, "%", &|_| rail));
                    rail_ends(ui, ends);
                });
                ui.end_row();
            } else if options.is_empty() {
                let label = match (d.kind.as_str(), k.as_str()) {
                    ("rectangularGrid", "rows") => tl!("Horizontal Dividers:").into(),
                    ("rectangularGrid", "columns") => tl!("Vertical Dividers:").into(),
                    ("polarGrid", "concentric") => tl!("Concentric Dividers:").into(),
                    ("polarGrid", "radial") => tl!("Radial Dividers:").into(),
                    _ => humanized(&k),
                };
                field(ui, d, &k, &label);
            } else {
                let row = row_label(ui, &humanized(&k));
                ui.label_inputs_by(row.id, |ui| dropdown(ui, d, &k, FIELD_W, &options, true));
                ui.end_row();
            }
        }
        more(ui, d);
    });
}

/// Command/effect parameters from the dialog fields (drop UI-only keys).
pub(super) fn params(d: &Dialog) -> Value {
    Value::Object(d.fields.iter().filter(|(k, _)| !k.starts_with("__") && k.as_str() != "preview").map(|(k, v)| (k.clone(), v.clone())).collect())
}

/// The (label, value) choices of a parameter that picks one of some values.
pub(super) type Choices = &'static [(&'static str, &'static str)];

/// Generic editor for command/effect parameters: numbers, booleans, strings and colours; the
/// parameters `is_length` names are distances shown and typed in `unit`, those `choices` gives
/// choices for are dropdowns. Fields come in `rank` order (by name among equals). Returns true when
/// a value changed.
pub(super) fn param_fields(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    is_length: &dyn Fn(&str) -> bool,
    choices: &dyn Fn(&str) -> Option<Choices>,
    rank: &dyn Fn(&str) -> usize,
    unit: Unit,
) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    egui::Grid::new("fxgrid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        let mut keys: Vec<(String, Value)> =
            d.fields.iter().filter(|(k, _)| !k.starts_with("__") && k.as_str() != "preview").map(|(k, v)| (k.clone(), v.clone())).collect();
        keys.sort_by_key(|(k, _)| rank(k));
        for (k, v) in keys {
            // A size's Relative / Absolute pair (Roughen, Zig Zag, Tweak) is its own label.
            let relative = k == "relative" && v.is_boolean();
            let row = ui.label(egui::RichText::new(if relative { String::new() } else { humanized(&k) }).color(t.text));
            // The row's actual label node names every input beside it, including plain
            // `TextEdit`s and the numeric field beside a slider (egui 0.37's strict a11y gate).
            ui.label_inputs_by(row.id, |ui| {
                if let Some(cur) = crate::widgets::blend_param(&k, &v) {
                    if let Some(m) = crate::widgets::blend_param_dropdown(ui, ("fx-blend", &k), cur) {
                        d.fields.insert(k.clone(), m);
                        changed = true;
                    }
                    return;
                }
                if is_length(&k) {
                    // An effect's documented range keeps the field to the distance it applies.
                    let range = vectorcraft_effects::length_range(&d.str("__effect"), &k);
                    changed |= length_within(ui, d, &k, unit, 140.0, range);
                    return;
                }
                if let Some(options) = choices(&k) {
                    let cur = v.as_str().unwrap_or_default();
                    let label = options.iter().find(|(_, value)| *value == cur).map_or(cur, |(l, _)| choice_label(l));
                    let labels: Vec<&str> = options.iter().map(|(l, _)| choice_label(l)).collect();
                    if let Some((_, value)) =
                        crate::widgets::dropdown_names(ui, ("fx-choice", &k), label, &labels, 140.0).and_then(|i| options.get(i))
                    {
                        d.fields.insert(k.clone(), json!(value));
                        changed = true;
                    }
                    return;
                }
                match v {
                    Value::Number(n) => {
                        if let Some(x) = crate::widgets::plain_field(ui, ("fx-num", &k), n.as_f64().unwrap_or(0.0), "", 3, 140.0) {
                            d.fields.insert(k.clone(), typed_number(x));
                            changed = true;
                        }
                    }
                    Value::Bool(b) if relative => {
                        ui.horizontal(|ui| {
                            for (label, on) in [(tl!("Relative"), true), (tl!("Absolute"), false)] {
                                if crate::widgets::radio(ui, label, b == on, true) && b != on {
                                    d.fields.insert(k.clone(), json!(on));
                                    changed = true;
                                }
                            }
                        });
                    }
                    Value::Bool(b) => {
                        if crate::widgets::check_box(ui, &humanized(&k), b, true) {
                            d.fields.insert(k.clone(), json!(!b));
                            changed = true;
                        }
                    }
                    Value::String(mut s) if s.contains('\n') => {
                        // Multi-line values (Graph Data CSV) get a text area.
                        if ui.add(egui::TextEdit::multiline(&mut s).desired_width(260.0).desired_rows(8).font(egui::TextStyle::Monospace)).changed() {
                            d.fields.insert(k.clone(), json!(s));
                            changed = true;
                        }
                    }
                    Value::String(mut s) => {
                        if ui.add(egui::TextEdit::singleline(&mut s).align(egui::Align2::LEFT_CENTER).desired_width(140.0)).changed() {
                            d.fields.insert(k.clone(), json!(s));
                            changed = true;
                        }
                    }
                    other => {
                        ui.label(egui::RichText::new(other.to_string()).color(t.text_dim).size(11.0));
                    }
                }
            });
            ui.end_row();
        }
    });
    changed
}

/// The label of one of [`param_fields`]' choices in the UI language, in the "effect" context: some
/// languages read the plain Soft, Horizontal and Vertical (Grain's types) as type settings.
pub(super) fn choice_label(label: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "effect", label)
}

/// A number typed in a command's dialog as JSON: a whole number as an integer, as counts are read
/// (`as_u64`: Object Mosaic's columns and rows, #1000), which `12.0` isn't; a fraction stays one.
/// Commands that take any number read either.
fn typed_number(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 1e15 { json!(x as i64) } else { json!(x) }
}

/// Editor for a plug-in's parameters from its manifest (the filter, live effect and exporter
/// dialogs): numbers within their range, whole numbers, checkboxes, dropdowns, text and lists of
/// numbers, a row each. A row is labelled as the manifest says (else from the parameter's name),
/// with its unit after the value and its help on the label and on a help button that opens its
/// page. Parameters outside any group come first, then each group as a section that folds
/// ([`Manifest::sections`]); a parameter whose `visibleWhen` the values don't meet is left out
/// ([`Manifest::visible`]). Returns true when a value changed. Labels, choices, groups and help are
/// the plug-in's text, shown as they are. A plug-in may declare more parameters
/// ([`MAX_PARAMS`](vectorcraft_plugins::manifest::MAX_PARAMS)) than the window has room for, and
/// the rows then scroll ([`super::modal::scrolled_rows`]).
pub(super) fn schema_fields(app: &mut crate::VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, m: &Manifest) -> bool {
    let (changed, help) = schema_rows(ui, d, m);
    if let Some(url) = help {
        app.open_url(&url);
    }
    changed
}

/// [`schema_fields`] → whether a value changed, and the help page asked for, left to the caller to
/// open.
pub(super) fn schema_rows(ui: &mut egui::Ui, d: &mut Dialog, m: &Manifest) -> (bool, Option<String>) {
    let mut changed = false;
    let mut help = None;
    let visible = m.visible(&d.fields);
    let sections = m.sections();
    // Every section's label column is as wide as the longest label shown, so the fields line up
    // down the dialog; the rows outside groups make up for the indent of a group's rows.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let label_width = (m.params.iter().filter(|(k, _)| visible.contains(k.as_str())))
        .map(|(k, p)| ui.painter().layout_no_wrap(param_label(k, p), font.clone(), egui::Color32::WHITE).size().x)
        .fold(0.0, f32::max);
    let indent = if sections.iter().any(|(group, _)| group.is_some()) { ui.spacing().indent } else { 0.0 };
    super::modal::scrolled_rows(ui, "plugin-fields", |ui| {
        for (section, (group, params)) in sections.into_iter().enumerate() {
            let shown: Vec<&(String, Param)> = params.into_iter().filter(|(k, _)| visible.contains(k.as_str())).collect();
            if shown.is_empty() {
                continue;
            }
            let rows = |ui: &mut egui::Ui, label_width: f32| {
                let grid = egui::Grid::new(("plugin-grid", section)).num_columns(2).spacing([12.0, 8.0]).min_col_width(label_width);
                grid.show(ui, |ui| {
                    for (k, p) in shown {
                        let (row_changed, opened) = param_row(ui, d, k, p);
                        changed |= row_changed;
                        help = help.take().or(opened);
                        ui.end_row();
                    }
                });
            };
            match group {
                None => rows(ui, label_width + indent),
                Some(g) => {
                    egui::CollapsingHeader::new(g).id_salt(("plugin-group", g)).default_open(true).show(ui, |ui| rows(ui, label_width));
                }
            }
        }
    });
    (changed, help)
}

/// The label of parameter `k`'s row: its manifest `label`, else its name spaced out.
fn param_label(k: &str, p: &Param) -> String {
    p.ui.label.as_ref().map_or_else(|| humanize(k), |l| format!("{l}:"))
}

/// One row of [`schema_fields`]: parameter `k`'s label and value field → whether the value changed,
/// and the help page asked for.
fn param_row(ui: &mut egui::Ui, d: &mut Dialog, k: &str, p: &Param) -> (bool, Option<String>) {
    let t = Tokens::get(ui.ctx());
    let label = param_label(k, p);
    let shown_label = ui.label(egui::RichText::new(&label).color(t.text));
    let shown_label = if let Some(help) = &p.ui.help { shown_label.on_hover_text(help) } else { shown_label };
    let cur = d.fields.get(k).cloned().unwrap_or_else(|| p.spec.default_value());
    let unit = p.ui.unit.as_deref().unwrap_or_default();
    let suffix = unit_suffix(unit);
    let mut opened = None;
    let new = ui
        .horizontal(|ui| {
            // The row's painted label names every input in its value column (egui 0.37's strict
            // accessibility gate), not only the first one.
            let (new, in_field) = ui.label_inputs_by(shown_label.id, |ui| value_field(ui, k, &label, &p.spec, &cur, &suffix));
            if !in_field && !unit.is_empty() {
                ui.label(egui::RichText::new(unit).color(t.text_dim));
            }
            if let Some(url) = &p.ui.help_url
                && crate::widgets::icon_button(ui, "external-link", tl!("Open this setting's help page"), false, 20.0).clicked()
            {
                opened = Some(url.clone());
            }
            new
        })
        .inner;
    let changed = new.map(|v| d.fields.insert(k.to_string(), v)).is_some();
    (changed, opened)
}

/// The field of parameter `k`'s value `cur` (a checkbox is called by the row's `label`) → the new
/// value when it changed, and whether the field shows `suffix` (numbers do; the other fields leave
/// the unit to a label after them).
fn value_field(ui: &mut egui::Ui, k: &str, label: &str, spec: &ParamSpec, cur: &Value, suffix: &str) -> (Option<Value>, bool) {
    match spec {
        ParamSpec::Number { min, max, .. } => {
            let x = cur.as_f64().unwrap_or(*min);
            (crate::widgets::range_field_named(ui, ("plugin-num", k), Some(label), x, *min..=*max, suffix, 2, 100.0).map(|x| json!(x)), true)
        }
        ParamSpec::Int { min, max, .. } => {
            let x = cur.as_i64().unwrap_or(*min).clamp(*min, *max);
            // An f64 holds every whole number up to 2^53, so the numeric field (arrows, wheel, arithmetic) is exact there.
            // Past it, the number typed is read as an integer.
            const EXACT: i64 = 1 << 53;
            if -EXACT <= *min && *max <= EXACT {
                let field =
                    crate::widgets::range_field_named(ui, ("plugin-int", k), Some(label), x as f64, *min as f64..=*max as f64, suffix, 0, 100.0);
                (field.map(|x| json!(x as i64)), true)
            } else {
                let shown = x.to_string();
                crate::widgets::take_dialog_focus(ui, ui.scope_id().with(("plugin-int", k)), &shown);
                let typed = crate::widgets::text_field_named(ui, ("plugin-int", k), Some(label), Some(&shown), 100.0, 1);
                (typed.and_then(|text| text.parse::<i64>().ok()).map(|x| json!(x.clamp(*min, *max))), false)
            }
        }
        ParamSpec::Bool { .. } => {
            let b = cur.as_bool().unwrap_or(false);
            (crate::widgets::check_box(ui, label, b, true).then(|| json!(!b)), false)
        }
        ParamSpec::Choice { options, .. } => {
            let labels: Vec<&str> = options.iter().map(String::as_str).collect();
            let chosen =
                crate::widgets::dropdown_names_named(ui, ("plugin-choice", k), Some(label), cur.as_str().unwrap_or_default(), &labels, 140.0);
            (chosen.and_then(|i| options.get(i)).map(|o| json!(o)), false)
        }
        ParamSpec::Text { max_length, .. } => {
            let typed = crate::widgets::text_field_named(ui, ("plugin-text", k), Some(label), cur.as_str(), 160.0, 1);
            (typed.map(|text| json!(text.chars().take(*max_length).collect::<String>())), false)
        }
        ParamSpec::NumberList { .. } => {
            let typed = crate::widgets::text_field_named(ui, ("plugin-list", k), Some(label), Some(&number_list_text(cur)), 160.0, 1);
            (typed.and_then(|text| read_number_list(&text, spec)), false)
        }
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

/// What a number field shows after its value for a plug-in parameter's `unit` (none: nothing): a
/// sign such as ° or % joins the number ("45°"), and any other unit is spaced from it ("0.4 mm",
/// "5 1/min"), so a unit starting with a digit isn't read as more of the number.
pub(crate) fn unit_suffix(unit: &str) -> String {
    if unit.is_empty() || unit.starts_with(['°', '%', '′', '″']) { unit.to_string() } else { format!(" {unit}") }
}

/// A number of a list as [`value_field`] shows it: at most 4 decimals, without trailing zeros.
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
        "Dx" => "Horizontal".into(),
        "Dy" => "Vertical".into(),
        "Sx" => "Horizontal %".into(),
        "Sy" => "Vertical %".into(),
        "Radius1" => "Radius 1".into(),
        "Radius2" => "Radius 2".into(),
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
        _ => format!("{s}:"),
    }
}

/// [`humanize`] translated where it is painted: the ":" stays outside the translated text.
pub(super) fn humanized(k: &str) -> String {
    let h = humanize(k);
    match h.strip_suffix(':') {
        Some(base) => format!("{}:", tl!(base)),
        None => tl!(&h).to_string(),
    }
}

/// Widths of [`slider`]'s label column and rail.
pub(super) const SLIDER_LABEL: f32 = 64.0;
pub(super) const SLIDER_WIDTH: f32 = 180.0;

/// A labelled slider with a value field for the number `d.fields[key]` in `range` (whole numbers,
/// `suffix` after the value); `track(t)` colours the rail at 0..1.
pub(super) fn slider(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    key: &str,
    label: &str,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    track: &dyn Fn(f32) -> egui::Color32,
) {
    slider_w(ui, d, (key, label, SLIDER_LABEL), range, suffix, track);
}

/// [`slider`] with its label column `label_w` wide: (`key`, `label`, `label_w`).
pub(super) fn slider_w(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    (key, label, label_w): (&str, &str, f32),
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    track: &dyn Fn(f32) -> egui::Color32,
) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_sized([label_w, 22.0], egui::Label::new(egui::RichText::new(tl!(label)).color(t.text)));
        slider_field_named(ui, d, key, tl!(label), range, suffix, track);
    });
}

/// The rail and value field of [`slider`] without its label (for a row that draws its own); screen
/// readers call the rail `name` (the row's label, already translated), with the number and text
/// the field shows as its value.
pub(super) fn slider_field_named(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    key: &str,
    name: &str,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    track: &dyn Fn(f32) -> egui::Color32,
) {
    let (min, max) = (*range.start(), *range.end());
    let v = d.f64(key, 0.0).clamp(min, max);
    let shown = crate::widgets::SliderValue::field(v, range.clone(), suffix, 0);
    slider_rail_valued(ui, d, key, name, range, Some(shown), track);
    if let Some(n) = crate::widgets::range_field_named(ui, ("dlg-field", key), Some(name), v, min..=max, suffix, 0, 52.0).filter(|n| *n != v) {
        d.fields.insert(key.into(), json!(n));
    }
}

/// The rail of [`slider_field_named`] alone: dragging it sets the number `d.fields[key]` to a
/// whole number in `range`, and its keys step the number while it has the keyboard focus
/// ([`crate::widgets::color_slider_stepped`]); a value past the range shows at its end until the
/// rail moves. Screen readers call it `name` (already translated), with the number
/// `d.fields[key]` as its value.
pub(super) fn slider_rail_named(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    key: &str,
    name: &str,
    range: std::ops::RangeInclusive<f64>,
    track: &dyn Fn(f32) -> egui::Color32,
) {
    let shown = crate::widgets::SliderValue::number(d.f64(key, 0.0), range.clone());
    slider_rail_valued(ui, d, key, name, range, Some(shown), track);
}

/// [`slider_rail_named`] that screen readers get `shown` from: the number and text of the field
/// the row draws beside it (no value for `None`, as for a blank field). Its keys step `shown`'s
/// number, which sets `d.fields[key]` to the number at the same place on `range` (`shown`'s range
/// and `range` are the same span in two units, such as millimeters and points).
pub(super) fn slider_rail_valued(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    key: &str,
    name: &str,
    range: std::ops::RangeInclusive<f64>,
    shown: Option<crate::widgets::SliderValue>,
    track: &dyn Fn(f32) -> egui::Color32,
) {
    let (min, max) = (*range.start(), *range.end());
    let held = d.f64(key, 0.0);
    let v = held.clamp(min, max);
    let units = shown.as_ref().map(|s| s.range.clone());
    let (dragged, _, keyed) =
        crate::widgets::color_slider_stepped(ui, ("dlg-slider", key), name, shown, ((v - min) / (max - min)) as f32, SLIDER_WIDTH, track);
    let keyed =
        keyed.zip(units).map(|(k, u)| if u == range { k } else { (min + (k - u.start()) / (u.end() - u.start()) * (max - min)).clamp(min, max) });
    if let Some(n) = dragged.map(|x| (min + x as f64 * (max - min)).round()).or(keyed).filter(|n| *n != held) {
        d.fields.insert(key.into(), json!(n));
    }
}

/// The words at the two ends of a slider's rail (Less … More), under a slider whose label column
/// is `label_w` wide.
pub(super) fn slider_ends(ui: &mut egui::Ui, label_w: f32, ends: (&str, &str)) {
    ui.horizontal(|ui| {
        ui.add_space(label_w + ui.spacing().item_spacing.x);
        rail_ends(ui, ends);
    });
}

/// [`slider_ends`] under a rail `width` wide that starts at the left edge of the row, for a slider
/// with its label on the line above.
pub(super) fn slider_ends_below(ui: &mut egui::Ui, width: f32, ends: (&str, &str)) {
    ui.horizontal(|ui| rail_ends_w(ui, width, ends));
}

/// The words of [`slider_ends`] under a rail that starts where the next widget does.
fn rail_ends(ui: &mut egui::Ui, ends: (&str, &str)) {
    rail_ends_w(ui, SLIDER_WIDTH, ends);
}

/// [`rail_ends`] under a rail `width` wide.
fn rail_ends_w(ui: &mut egui::Ui, width: f32, (left, right): (&str, &str)) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::hover());
    let font = egui::FontId::proportional(11.0);
    ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, left, font.clone(), t.text_dim);
    ui.painter().text(r.right_center(), egui::Align2::RIGHT_CENTER, right, font, t.text_dim);
}

/// The field where [`preview`] keeps the parameters it last previewed.
pub(super) const PREVIEWED: &str = "__previewed";

/// The Preview checkbox of a dialog that previews `cmd` on the canvas: while it is on, `params` run
/// again on the interaction's snapshot whenever they differ from the last preview (whether a widget
/// or `ui.dialog.set` changed them); turning it off rolls back.
pub(super) fn preview(app: &mut crate::VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, label: &str, cmd: &str, params: Value) {
    let _ = preview_result(app, ui, d, label, cmd, params);
}

/// As [`preview`], returning the command's result only when a new preview succeeds.
/// A dialog can retain a small summary (such as point counts) in its private fields.
pub(super) fn preview_result(
    app: &mut crate::VectorcraftApp,
    ui: &mut egui::Ui,
    d: &mut Dialog,
    label: &str,
    cmd: &str,
    params: Value,
) -> Option<Value> {
    const LAST: &str = PREVIEWED;
    ui.add_space(8.0);
    let mut on = d.bool("preview");
    if crate::widgets::check(ui, tl!("Preview"), on, true) {
        on = !on;
        d.fields.insert("preview".into(), json!(on));
    }
    if !on {
        if d.fields.remove(LAST).is_some() {
            let _ = app.session.cancel_interaction();
        }
        return None;
    }
    preview_now(app, d, label, cmd, params)
}

/// The preview of [`preview_result`] without its checkbox: `params` run again on the interaction's
/// snapshot when they differ from the last preview. Returns the command's result only when a new
/// preview succeeds.
pub(super) fn preview_now(app: &mut crate::VectorcraftApp, d: &mut Dialog, label: &str, cmd: &str, params: Value) -> Option<Value> {
    let mut result = None;
    if d.fields.get(PREVIEWED) != Some(&params) || !app.session.in_interaction() {
        let _ = app.session.begin_interaction(label);
        match app.session.preview(cmd, &params) {
            Ok(r) => result = Some(r),
            Err(e) => app.status(e.to_string()),
        }
        d.fields.insert(PREVIEWED.into(), params);
    }
    result
}

/// OK of a dialog using [`preview`]: keep the previewed `cmd` as one undo step (or run it when no
/// preview runs) and close; the command's warning shows in the status bar either way. On an error
/// the dialog stays open.
pub(super) fn commit_preview(app: &mut crate::VectorcraftApp, cmd: &str, params: Value) -> Result<Value, String> {
    let r = if app.session.in_interaction() {
        match app.session.preview(cmd, &params) {
            Ok(v) => {
                let r = app.session.commit_interaction().map(|_| Value::Null).map_err(|e| e.to_string());
                if r.is_ok()
                    && let Some(w) = v.get("warning").and_then(Value::as_str)
                {
                    app.status(w.to_string());
                }
                r
            }
            Err(e) => Err(e.to_string()),
        }
    } else {
        app.run(cmd, params)
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// The `[top, bottom, left, right]` bleed in `d.fields["bleed"]` (points).
pub(super) fn bleed_values(d: &Dialog) -> [f64; 4] {
    let mut out = [0.0; 4];
    if let Some(a) = d.fields.get("bleed").and_then(Value::as_array) {
        for (o, v) in out.iter_mut().zip(a) {
            *o = v.as_f64().unwrap_or(0.0);
        }
    }
    out
}

/// Bleed fields (Document Setup, New Document): Top, Bottom, Left and Right shown in `unit`, each
/// `width` wide with a caption above, and a link toggle (`bleedLinked`, on by default) that keeps
/// them equal. Values are clamped to 0–72 pt.
pub(super) fn bleed(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit, width: f32) {
    let t = Tokens::get(ui.ctx());
    let mut b = bleed_values(d);
    let linked = d.fields.get("bleedLinked").and_then(Value::as_bool).unwrap_or(true);
    let mut changed = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (i, side) in ["Top", "Bottom", "Left", "Right"].into_iter().enumerate() {
            ui.vertical(|ui| {
                let label = ui.label(egui::RichText::new(tl!(side)).size(11.0).color(t.text_dim));
                let v = ui.label_inputs_by(label.id, |ui| crate::widgets::num_field(ui, ("bleed", i), Some(b[i]), unit, width));
                if let Some(v) = v {
                    changed = Some((i, v.clamp(0.0, vectorcraft_doc::setup::MAX_BLEED)));
                }
            });
        }
        ui.vertical(|ui| {
            ui.add_space(15.0);
            let tip = if linked { tl!("Make the bleed values differ") } else { tl!("Make all bleed settings the same") };
            if crate::widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tip, linked, 24.0).clicked() {
                d.fields.insert("bleedLinked".into(), json!(!linked));
                if !linked {
                    // Linking makes every side the top's.
                    changed = Some((0, b[0]));
                }
            }
        });
    });
    if let Some((i, v)) = changed {
        // Linked (also just now): every side takes the value.
        if d.fields.get("bleedLinked").and_then(Value::as_bool).unwrap_or(true) {
            b = [v; 4];
        } else {
            b[i] = v;
        }
        d.fields.insert("bleed".into(), json!(b));
    }
}

/// A small grey caption above a field (New Document's details column).
pub(super) fn caption(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(text)).size(11.0).color(t.text_dim));
}

#[cfg(test)]
mod plugin_field_tests {
    use super::*;

    /// A unit reads after its number as it is written: a sign joins it ("45°", "50%"), anything
    /// else is spaced, so a unit starting with a digit isn't read as more of the number.
    #[test]
    fn units_follow_the_number() {
        let shown = |unit| format!("5{}", unit_suffix(unit));
        assert_eq!([shown("mm"), shown("°"), shown("%"), shown("1/min"), shown("µm")], ["5 mm", "5°", "5%", "5 1/min", "5 µm"]);
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

#[cfg(test)]
mod selected_text_tests {
    use super::*;
    use crate::widgets::look_tests::{frame, glyph_colours, themed};

    // Regression test for #1465: selected text in a dialog's text field was in the accent, on a
    // selection tinted with the same accent. It is in the strong text colour.
    #[test]
    fn selected_text_in_a_dialog_text_field_is_in_the_strong_text_colour() {
        for brightness in crate::theme::Brightness::ALL {
            let ctx = themed(brightness);
            let t = Tokens::get(&ctx);
            let mut d = Dialog::new("test", json!({"name": "Artboard 1"}));
            let id = std::cell::Cell::new(egui::Id::NULL);
            let mut add = |ui: &mut egui::Ui| {
                id.set(ui.scope_id().with(("dlg-text", "name")));
                text_edit(ui, &mut d, "name", 120.0);
            };
            frame(&ctx, 0.0, vec![], &mut add);
            // Focus the field and select all of its text.
            ctx.memory_mut(|m| m.request_focus(id.get()));
            frame(&ctx, 0.1, vec![], &mut add);
            let mut state = egui::TextEdit::load_state(&ctx, id.get()).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(10))));
            state.store(&ctx, id.get());
            let shapes = frame(&ctx, 0.2, vec![], &mut add);
            let colours = glyph_colours(&shapes);
            assert!(!colours.is_empty(), "{brightness:?}: the text is drawn");
            let mut used = colours.clone();
            used.dedup();
            assert_eq!(used, [t.text_strong], "{brightness:?}: selected text in {used:?}");
        }
    }
}
