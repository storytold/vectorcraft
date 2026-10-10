//! Field widgets shared by the dialogs: labelled text fields, checkboxes, the generic field grid
//! and the typed parameter editor of the command and effect dialogs.

use serde_json::{Value, json};
use vectorcraft_doc::Unit;

use crate::state::Dialog;
use crate::theme::Tokens;

/// Width of a dialog's value fields ([`text`] and [`length`], frame included).
const FIELD_W: f32 = 132.0;

/// A labelled text field bound to `d.fields[key]` (one grid row).
pub(super) fn field(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    field_w(ui, d, key, label, FIELD_W - 12.0);
}

/// [`field`] `width` points wide.
pub(super) fn field_w(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, width: f32) {
    row_label(ui, label);
    text(ui, d, key, width);
    ui.end_row();
}

/// The label cell of a grid row.
fn row_label(ui: &mut egui::Ui, label: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(label)).color(t.text_dim));
}

/// A labelled [`length`] field (one grid row).
pub(super) fn length_field(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, unit: Unit) {
    row_label(ui, label);
    length(ui, d, key, unit, FIELD_W);
    ui.end_row();
}

/// A distance field `width` wide bound to `d.fields[key]`: shown and typed in `unit` (a typed unit,
/// "5 mm", wins), kept in points (a string with a unit, "12 pt", also reads). Returns true when it
/// changed.
pub(super) fn length(ui: &mut egui::Ui, d: &mut Dialog, key: &str, unit: Unit, width: f32) -> bool {
    let value = d.fields.contains_key(key).then(|| d.f64(key, 0.0));
    let Some(v) = crate::widgets::num_field(ui, ("dlg-len", key), value, unit, width) else { return false };
    d.fields.insert(key.into(), json!(v));
    true
}

/// The fields of the form dialogs ([`grid`]) that are distances, by dialog kind.
fn lengths(kind: &str) -> &'static [&'static str] {
    match kind {
        "move" => &["dx", "dy"],
        "rectangle" | "ellipse" | "artboardOptions" => &["width", "height"],
        "roundedRectangle" => &["width", "height", "radius"],
        "polygon" => &["radius"],
        "star" => &["radius1", "radius2"],
        "lineSegment" => &["length"],
        "offsetPath" => &["offset"],
        "simplify" => &["tolerance"],
        "splitIntoGrid" => &["gutter"],
        _ => &[],
    }
}

/// The fields of the form dialogs ([`grid`]) that come first, in this order (the others follow by
/// name).
fn order(kind: &str) -> &'static [&'static str] {
    match kind {
        "offsetPath" => &["offset", "joins", "miterLimit"],
        "splitIntoGrid" => &["rows", "columns", "gutter"],
        "artboardOptions" => &["name", "width", "height"],
        _ => &[],
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
        ("reflect" | "shear", "axis") => vec![axis("horizontal", "Horizontal"), axis("vertical", "Vertical")],
        _ => vec![],
    }
}

/// A text field `width` wide bound to `d.fields[key]`. Returns true when it changed.
pub(super) fn text(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) -> bool {
    text_edit(ui, d, key, width).changed()
}

/// [`text`] returning the field's response (focus, Enter).
pub(super) fn text_edit(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut s = d.str(key);
    let id = ui.id().with(("dlg-text", key));
    crate::widgets::take_dialog_focus(ui, id, &s);
    let r = egui::Frame::NONE
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| ui.add(egui::TextEdit::singleline(&mut s).id(id).frame(egui::Frame::NONE).desired_width(width)));
    if r.inner.changed() {
        d.fields.insert(key.into(), Value::String(s));
    }
    r.inner
}

/// A text box `rows` lines tall and `width` wide bound to `d.fields[key]`. Enter adds a line (it
/// doesn't press OK while the box has focus).
pub(super) fn text_area(ui: &mut egui::Ui, d: &mut Dialog, key: &str, width: f32, rows: usize) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let mut s = d.str(key);
    let r = egui::Frame::NONE
        .fill(t.input)
        .stroke(egui::Stroke::new(1.0, t.input_border))
        .corner_radius(egui::CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 3))
        .show(ui, |ui| ui.add(egui::TextEdit::multiline(&mut s).frame(egui::Frame::NONE).desired_width(width).desired_rows(rows)));
    if r.inner.changed() {
        d.fields.insert(key.into(), Value::String(s));
    }
    if r.inner.has_focus() {
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
    r.inner
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

/// The generic dialog body: a field per non-boolean value (positions, indices and UI-only keys
/// hidden) in the kind's [`order`], the distances in `unit`, the [`choices`] as dropdowns.
pub(super) fn grid(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    let (lengths, first) = (lengths(&d.kind), order(&d.kind));
    let mut keys: Vec<String> = d
        .fields
        .iter()
        .filter(|(k, v)| !matches!(k.as_str(), "x" | "y" | "origin" | "index") && !k.starts_with("__") && !v.is_boolean())
        .map(|(k, _)| k.clone())
        .collect();
    // Stable: the fields `order` doesn't name keep their order after the ones it does.
    keys.sort_by_key(|k| first.iter().position(|f| f == k).unwrap_or(first.len()));
    egui::Grid::new("dlg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        for k in keys {
            let options = choices(&d.kind, &k);
            if lengths.contains(&k.as_str()) {
                length_field(ui, d, &k, &humanized(&k), unit);
            } else if options.is_empty() {
                field(ui, d, &k, &humanized(&k));
            } else {
                row_label(ui, &humanized(&k));
                dropdown(ui, d, &k, FIELD_W, &options, true);
                ui.end_row();
            }
        }
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
            ui.label(egui::RichText::new(if relative { String::new() } else { humanized(&k) }).color(t.text));
            if let Some(cur) = crate::widgets::blend_param(&k, &v) {
                if let Some(m) = crate::widgets::blend_param_dropdown(ui, ("fx-blend", &k), cur) {
                    d.fields.insert(k, m);
                    changed = true;
                }
                ui.end_row();
                continue;
            }
            if is_length(&k) {
                changed |= length(ui, d, &k, unit, 140.0);
                ui.end_row();
                continue;
            }
            if let Some(options) = choices(&k) {
                // In the "effect" context: some languages read the plain Soft, Horizontal and
                // Vertical (Grain's types) as type settings.
                let shown = |label: &'static str| crate::i18n::tr_ctx(crate::i18n::current(), "effect", label);
                let cur = v.as_str().unwrap_or_default();
                let label = options.iter().find(|(_, value)| *value == cur).map_or(cur, |(l, _)| shown(l));
                let labels: Vec<&str> = options.iter().map(|(l, _)| shown(l)).collect();
                if let Some((_, value)) = crate::widgets::dropdown_names(ui, ("fx-choice", &k), label, &labels, 140.0).and_then(|i| options.get(i)) {
                    d.fields.insert(k, json!(value));
                    changed = true;
                }
                ui.end_row();
                continue;
            }
            match v {
                Value::Number(n) => {
                    if let Some(x) = crate::widgets::plain_field(ui, ("fx-num", &k), n.as_f64().unwrap_or(0.0), "", 3, 140.0) {
                        d.fields.insert(k, json!(x));
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
                    if crate::widgets::check(ui, "", b, true) {
                        d.fields.insert(k, json!(!b));
                        changed = true;
                    }
                }
                Value::String(mut s) if s.contains('\n') => {
                    // Multi-line values (Graph Data CSV) get a text area.
                    if ui.add(egui::TextEdit::multiline(&mut s).desired_width(260.0).desired_rows(8).font(egui::TextStyle::Monospace)).changed() {
                        d.fields.insert(k, json!(s));
                        changed = true;
                    }
                }
                Value::String(mut s) => {
                    if ui.add(egui::TextEdit::singleline(&mut s).desired_width(140.0)).changed() {
                        d.fields.insert(k, json!(s));
                        changed = true;
                    }
                }
                other => {
                    ui.label(egui::RichText::new(other.to_string()).color(t.text_dim).size(11.0));
                }
            }
            ui.end_row();
        }
    });
    changed
}

/// Editor for a plug-in's parameters from its schema (plug-in filter and effect dialogs): numbers
/// within their range, whole numbers, checkboxes and dropdowns, in declaration order. Returns
/// true when a value changed. The labels (from the parameter keys) and choices are the plug-in's
/// text, shown as they are.
pub(super) fn schema_fields(ui: &mut egui::Ui, d: &mut Dialog, specs: &[(String, vectorcraft_plugins::ParamSpec)]) -> bool {
    use vectorcraft_plugins::ParamSpec;
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    egui::Grid::new("plugin-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        for (k, spec) in specs {
            ui.label(egui::RichText::new(humanize(k)).color(t.text));
            let cur = d.fields.get(k).cloned().unwrap_or_else(|| spec.default_value());
            let new = match spec {
                ParamSpec::Number { min, max, .. } => {
                    let x = cur.as_f64().unwrap_or(*min);
                    crate::widgets::range_field(ui, ("plugin-num", k), x, *min..=*max, "", 2, 100.0).map(|x| json!(x))
                }
                ParamSpec::Int { min, max, .. } => {
                    let x = cur.as_i64().unwrap_or(*min) as f64;
                    crate::widgets::range_field(ui, ("plugin-int", k), x, *min as f64..=*max as f64, "", 0, 100.0).map(|x| json!(x as i64))
                }
                ParamSpec::Bool { .. } => {
                    let mut b = cur.as_bool().unwrap_or(false);
                    ui.checkbox(&mut b, "").changed().then(|| json!(b))
                }
                ParamSpec::Choice { options, .. } => {
                    let labels: Vec<&str> = options.iter().map(String::as_str).collect();
                    crate::widgets::dropdown_names(ui, ("plugin-choice", k), cur.as_str().unwrap_or_default(), &labels, 140.0)
                        .and_then(|i| options.get(i))
                        .map(|o| json!(o))
                }
            };
            if let Some(v) = new {
                d.fields.insert(k.clone(), v);
                changed = true;
            }
            ui.end_row();
        }
    });
    changed
}

/// A field label from its camelCase key (`miterLimit` → "Miter Limit:").
pub(super) fn humanize(k: &str) -> String {
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
        "Channel1" => "Channel 1:".into(),
        "Channel2" => "Channel 2:".into(),
        "Channel3" => "Channel 3:".into(),
        "Channel4" => "Channel 4:".into(),
        "Include Cmy Blacks" => "Include Blacks with CMY:".into(),
        "Align To Path" => "Align to Path:".into(),
        "Create" => "Create New Fields by:".into(),
        _ => format!("{s}:"),
    }
}

/// [`humanize`] translated where it is painted: the ":" stays outside the translated text.
fn humanized(k: &str) -> String {
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
        slider_field(ui, d, key, range, suffix, track);
    });
}

/// The rail and value field of [`slider`] without its label (for a row that draws its own).
pub(super) fn slider_field(
    ui: &mut egui::Ui,
    d: &mut Dialog,
    key: &str,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    track: &dyn Fn(f32) -> egui::Color32,
) {
    let (min, max) = (*range.start(), *range.end());
    let v = d.f64(key, 0.0).clamp(min, max);
    slider_rail(ui, d, key, range, track);
    if let Some(n) = crate::widgets::plain_field(ui, ("dlg-field", key), v, suffix, 0, 52.0).map(|x| x.round().clamp(min, max)).filter(|n| *n != v) {
        d.fields.insert(key.into(), json!(n));
    }
}

/// The rail of [`slider_field`] alone: dragging it sets the number `d.fields[key]` to a whole
/// number in `range` (a value past the range shows at its end until the rail moves).
pub(super) fn slider_rail(ui: &mut egui::Ui, d: &mut Dialog, key: &str, range: std::ops::RangeInclusive<f64>, track: &dyn Fn(f32) -> egui::Color32) {
    let (min, max) = (*range.start(), *range.end());
    let v = d.f64(key, 0.0).clamp(min, max);
    let (dragged, _) = crate::widgets::color_slider(ui, ("dlg-slider", key), ((v - min) / (max - min)) as f32, SLIDER_WIDTH, track);
    if let Some(n) = dragged.map(|x| (min + x as f64 * (max - min)).round()).filter(|n| *n != v) {
        d.fields.insert(key.into(), json!(n));
    }
}

/// The words at the two ends of a slider's rail (Less … More), under a slider whose label column
/// is `label_w` wide.
pub(super) fn slider_ends(ui: &mut egui::Ui, label_w: f32, (left, right): (&str, &str)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(label_w + ui.spacing().item_spacing.x);
        let (r, _) = ui.allocate_exact_size(egui::vec2(SLIDER_WIDTH, 14.0), egui::Sense::hover());
        let font = egui::FontId::proportional(11.0);
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, left, font.clone(), t.text_dim);
        ui.painter().text(r.right_center(), egui::Align2::RIGHT_CENTER, right, font, t.text_dim);
    });
}

/// The field where [`preview`] keeps the parameters it last previewed.
pub(super) const PREVIEWED: &str = "__previewed";

/// The Preview checkbox of a dialog that previews `cmd` on the canvas: while it is on, `params` run
/// again on the interaction's snapshot whenever they differ from the last preview (whether a widget
/// or `ui.dialog.set` changed them); turning it off rolls back.
pub(super) fn preview(app: &mut crate::VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog, label: &str, cmd: &str, params: Value) {
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
        return;
    }
    if d.fields.get(LAST) != Some(&params) || !app.session.in_interaction() {
        let _ = app.session.begin_interaction(label);
        if let Err(e) = app.session.preview(cmd, &params) {
            app.status(e.to_string());
        }
        d.fields.insert(LAST.into(), params);
    }
}

/// OK of a dialog using [`preview`]: keep the previewed `cmd` as one undo step (or run it when no
/// preview runs) and close. On an error the dialog stays open.
pub(super) fn commit_preview(app: &mut crate::VectorcraftApp, cmd: &str, params: Value) -> Result<Value, String> {
    let r = if app.session.in_interaction() {
        match app.session.preview(cmd, &params) {
            Ok(_) => app.session.commit_interaction().map(|_| Value::Null).map_err(|e| e.to_string()),
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
                ui.label(egui::RichText::new(tl!(side)).size(11.0).color(t.text_dim));
                if let Some(v) = crate::widgets::num_field(ui, ("bleed", i), Some(b[i]), unit, width) {
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
