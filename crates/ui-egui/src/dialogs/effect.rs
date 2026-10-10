//! Live effect dialogs (Effect menu, Appearance panel): the effect's parameters (`__effect`, headed
//! `__label`) with a live Preview that OK keeps as one undo step and Cancel rolls back. `__item`,
//! when present, is the appearance item the effect goes to (else the Appearance panel's active
//! item applies). With `__index` the dialog edits that applied effect in place
//! (`effect.setParams`) instead of adding one (`effect.apply`).
//!
//! Choosing an effect that the target already carries first asks ([`EXISTS`]) whether to edit it
//! or add another one.

use serde_json::{Map, Value, json};

use vectorcraft_color::Color;

use super::{DialogSpec, form, transform_each};
use crate::panels::c32;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| heading_label(d), body, confirm, preview: true, ..DialogSpec::FORM };

/// The effect's name (`__label`, English) in the UI language: the catalog may carry it with or
/// without its trailing ellipsis. A plug-in effect's name is shown as it is.
fn heading_label(d: &Dialog) -> String {
    let label = d.str("__label");
    if vectorcraft_plugins::effect::plugin_id(&d.str("__effect")).is_some() {
        return label;
    }
    let dotted = format!("{label}…");
    let shown = tl!(&dotted);
    if shown != dotted { shown.trim_end_matches('…').to_string() } else { tl!(&label).to_string() }
}

/// The dialog kind asking whether to edit an effect that is already applied or add another.
/// Fields: `__effect`, `__label`, `__item?` (as given to `effect.dialog`), `__target` and
/// `__index` (the applied effect's item and position). OK edits it; `discard: true` then OK adds
/// a new one.
pub const EXISTS: &str = "effectExists";

pub(super) const EXISTS_SPEC: DialogSpec = DialogSpec {
    heading: |d| crate::i18n::fmt(tl!("{effect} is already applied"), &[("effect", &heading_label(d))]),
    body: |_, ui, _| {
        ui.label(egui::RichText::new(tl!("Edit the applied effect, or add another one?")).color(Tokens::get(ui.ctx()).text_dim));
        false
    },
    confirm: confirm_exists,
    ok: Some("Edit"),
    discard: Some("Add New"),
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// `effect.dialog {effect, index?, item?}`: edit applied effect `index` of `item` (null: the
/// object's effects; omitted: the active item's, else the object's) prefilled with its values,
/// or add `effect`, first asking when that list already has it.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("effect").and_then(Value::as_str).unwrap_or_default();
    let info = vectorcraft_effects::effect_info(id).ok_or_else(|| format!("unknown effect `{id}`"))?;
    // The list the effect goes to: `item` as given, else the active item's (the object's without
    // one). A new effect keeps `item` unresolved, so `effect.apply` targets each object as usual.
    let target = p.get("item").cloned().unwrap_or_else(|| json!(app.session.appearance_item()));
    let applied = target_effects(app, &target);
    let mut fields = Map::new();
    fields.insert("__effect".into(), json!(id));
    fields.insert("__label".into(), json!(info.label.trim_end_matches('…')));
    if let Some(item) = p.get("item") {
        fields.insert("__item".into(), item.clone());
    }
    match p.get("index").and_then(Value::as_u64) {
        Some(k) => {
            let e = applied.get(k as usize).ok_or_else(|| format!("no effect at index {k}"))?;
            if e.id != id {
                return Err(format!("effect {k} is `{}`, not `{id}`", e.id));
            }
            fields.insert("__item".into(), target);
            fields.insert("__index".into(), json!(k));
            fields.extend(params_of(e));
        }
        None => {
            if let Some(k) = applied.iter().rposition(|e| e.id == id) {
                fields.insert("__target".into(), target);
                fields.insert("__index".into(), json!(k));
                app.ui.dialog = Some(Dialog { kind: EXISTS.into(), fields });
                return Ok(json!({ "pending": EXISTS }));
            }
            fields.extend(info.defaults.as_object().cloned().unwrap_or_default());
        }
    }
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: "effect".into(), fields });
    Ok(Value::Null)
}

/// The effects of the first selected object that `item` (an index or null) addresses.
fn target_effects(app: &VectorcraftApp, item: &Value) -> Vec<vectorcraft_doc::Effect> {
    let Some(st) = app.session.active() else { return vec![] };
    let node = st.selection.objects.first().and_then(|id| st.doc.node(*id));
    node.and_then(|n| n.appearance.effects_at(item.as_u64().map(|i| i as usize))).cloned().unwrap_or_default()
}

/// An applied effect's values over its defaults.
fn params_of(e: &vectorcraft_doc::Effect) -> Map<String, Value> {
    match vectorcraft_effects::merged_params(&e.id, &e.params) {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// OK in [`EXISTS`]: the dialog editing the applied effect, or (`discard`) a fresh one.
fn confirm_exists(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut fields = d.fields.clone();
    fields.remove("discard");
    let target = fields.remove("__target").unwrap_or(Value::Null);
    if d.bool("discard") {
        fields.remove("__index");
        let defaults = vectorcraft_effects::default_params(&d.str("__effect"));
        fields.extend(defaults.and_then(|v| v.as_object().cloned()).unwrap_or_default());
    } else {
        let k = fields.get("__index").and_then(Value::as_u64).unwrap_or(0) as usize;
        fields.extend(target_effects(app, &target).get(k).map(params_of).unwrap_or_default());
        fields.insert("__item".into(), target);
    }
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: "effect".into(), fields });
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (id, relative) = (d.str("__effect"), d.bool("relative"));
    // Plug-in effects get fields from their parameter schema; colour adjustments their sliders.
    let changed = match vectorcraft_plugins::effect::installed(&id) {
        Some(plugin) => form::schema_fields(ui, d, &plugin.manifest().params),
        None if vectorcraft_effects::is_adjustment(&id) => adjust_fields(ui, d),
        None if id == TRANSFORM => transform_fields(ui, d, app.session.general_unit()),
        None => {
            let doc = vectorcraft_effects::effect_info(&id).map(|e| e.params).unwrap_or_default();
            let unit = app.session.general_unit();
            form::param_fields(ui, d, &|k| vectorcraft_effects::is_length(&id, k, relative), &|k| choices(&id, k), &|k| doc_rank(doc, k), unit)
        }
    };
    ui.add_space(6.0);
    let pv_changed = widgets::check(ui, tl!("Preview"), d.bool("preview"), true);
    let pv = d.bool("preview") != pv_changed;
    d.fields.insert("preview".into(), json!(pv));
    if pv && (changed || pv_changed || !app.session.in_interaction()) {
        let label = d.str("__label");
        let _ = app.session.begin_interaction(&label);
        let (cmd, p) = command(d);
        let _ = app.session.preview(cmd, &p);
    } else if !pv && pv_changed {
        let _ = app.session.cancel_interaction();
    }
    false
}

/// The parameters of the effect dialogs that pick one of some values, as (label, value).
fn choices(effect: &str, key: &str) -> Option<form::Choices> {
    match (effect, key) {
        ("blur.radial", "method") => Some(&[("Spin", "spin"), ("Zoom", "zoom")]),
        ("blur.radial", "quality") => Some(&[("Draft", "draft"), ("Good", "good"), ("Best", "best")]),
        ("blur.smart", "quality") => Some(&[("Low", "low"), ("Medium", "medium"), ("High", "high")]),
        ("pixelate.mezzotint", "type") => Some(&vectorcraft_effects::pixel::MEZZOTINT_TYPES),
        ("texture.grain", "grainType") => Some(&vectorcraft_effects::pixel::GRAIN_TYPES),
        ("texture.texturizer", "texture") => Some(&vectorcraft_effects::pixel::TEXTURES),
        ("texture.texturizer", "lightDirection") => Some(&vectorcraft_effects::pixel::LIGHT_DIRECTIONS),
        ("video.deinterlace", "eliminate") => Some(&[("Odd Fields", "odd"), ("Even Fields", "even")]),
        ("video.deinterlace", "create") => Some(&[("Duplication", "duplication"), ("Interpolation", "interpolation")]),
        ("stylize.innerGlow", "source") => Some(&[("Center", "center"), ("Edge", "edge")]),
        ("path.offsetPath", "joins") => Some(&[("Miter", "miter"), ("Round", "round"), ("Bevel", "bevel")]),
        ("distort.roughen" | "distort.zigZag", "points") => Some(&[("Smooth", "smooth"), ("Corner", "corner")]),
        _ => None,
    }
}

/// Where parameter `key` comes in the effect's parameter documentation `doc` (`{radius: …,
/// threshold: …}`), which lists them in its dialog's order; the end when it isn't there.
pub(super) fn doc_rank(doc: &str, key: &str) -> usize {
    let starts = |at: usize| doc.get(..at).and_then(|s| s.chars().last()).is_some_and(|c| c == '{' || c == ' ');
    doc.match_indices(key)
        .map(|(at, _)| at)
        .find(|at| starts(*at) && doc.get(at + key.len()..).is_some_and(|rest| rest.starts_with(':') || rest.starts_with("?:")))
        .unwrap_or(usize::MAX)
}

/// A slider of a colour adjustment dialog: (effect, field, label, range, rail colour at 0..1).
type Slider = (&'static str, &'static str, &'static str, (f64, f64), fn(f32) -> egui::Color32);

fn rgb(r: f32, g: f32, b: f32) -> egui::Color32 {
    c32(&Color::rgb(r, g, b))
}

fn grey_rail(x: f32) -> egui::Color32 {
    rgb(x, x, x)
}

fn hue_rail(x: f32) -> egui::Color32 {
    c32(&Color::from_hsb(((x - 0.5) * 360.0).rem_euclid(360.0), 0.8, 0.9))
}

fn saturation_rail(x: f32) -> egui::Color32 {
    c32(&Color::from_hsb(0.0, x, 0.9))
}

fn temperature_rail(x: f32) -> egui::Color32 {
    rgb(0.35 + 0.65 * x, 0.55 + 0.05 * x, 1.0 - 0.8 * x)
}

fn tint_rail(x: f32) -> egui::Color32 {
    rgb(0.3 + 0.6 * x, 0.85 - 0.55 * x, 0.3 + 0.6 * x)
}

const SLIDERS: [Slider; 12] = [
    ("adjust.brightnessContrast", "brightness", "Brightness", (-100.0, 100.0), grey_rail),
    ("adjust.brightnessContrast", "contrast", "Contrast", (-100.0, 100.0), grey_rail),
    ("adjust.hueSaturation", "hue", "Hue", (-180.0, 180.0), hue_rail),
    ("adjust.hueSaturation", "saturation", "Saturation", (-100.0, 100.0), saturation_rail),
    ("adjust.hueSaturation", "lightness", "Lightness", (-100.0, 100.0), grey_rail),
    ("adjust.levels", "inputBlack", "Input Black", (0.0, 255.0), grey_rail),
    ("adjust.levels", "inputWhite", "Input White", (0.0, 255.0), grey_rail),
    ("adjust.levels", "outputBlack", "Output Black", (0.0, 255.0), grey_rail),
    ("adjust.levels", "outputWhite", "Output White", (0.0, 255.0), grey_rail),
    ("adjust.shiftToColor", "amount", "Amount", (0.0, 100.0), grey_rail),
    ("adjust.temperatureTint", "temperature", "Temperature", (-100.0, 100.0), temperature_rail),
    ("adjust.temperatureTint", "tint", "Tint", (-100.0, 100.0), tint_rail),
];

/// The colour adjustment dialogs (Effect → Color Adjustments): a slider per amount, then the
/// channel as a menu and the other values (Curves' points, Levels' gamma, the target colour) in
/// the same label column, and the options. Returns whether the values differ from those last
/// previewed.
fn adjust_fields(ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    const LABEL_W: f32 = 84.0;
    let id = d.str("__effect");
    for (_, key, label, (min, max), rail) in SLIDERS.iter().filter(|s| s.0 == id) {
        form::slider_w(ui, d, (key, label, LABEL_W), *min..=*max, "", rail);
    }
    if d.fields.contains_key("channel") {
        form::choice(
            ui,
            d,
            "channel",
            tl!("Channel"),
            (LABEL_W, 120.0),
            &[("rgb", "RGB"), ("red", tl!("Red")), ("green", tl!("Green")), ("blue", tl!("Blue"))],
        );
    }
    if d.fields.contains_key("points") {
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            curve_graph(ui, d);
        });
    }
    for (key, label, width) in [("points", tl!("Points"), 220.0), ("color", tl!("Color"), 120.0)] {
        if d.fields.contains_key(key) {
            widgets::label_row(ui, label, LABEL_W, |ui| {
                form::text(ui, d, key, width);
            });
        }
    }
    if d.fields.contains_key("gamma") {
        widgets::label_row(ui, tl!("Gamma"), LABEL_W, |ui| {
            if let Some(g) = widgets::range_field(ui, "gamma", d.f64("gamma", 1.0), 0.1..=10.0, "", 2, 60.0) {
                d.fields.insert("gamma".into(), json!(g));
            }
        });
    }
    for (key, label) in [("colorize", tl!("Colorize")), ("preserveLightness", tl!("Preserve Lightness"))] {
        if d.fields.contains_key(key) {
            form::check(ui, d, key, label);
        }
    }
    values_changed(d)
}

/// Do the dialog's values differ from those last previewed? (Remembers them for the next frame.)
fn values_changed(d: &mut Dialog) -> bool {
    let params = form::params(d);
    let changed = d.fields.get(form::PREVIEWED) != Some(&params);
    d.fields.insert(form::PREVIEWED.into(), params);
    changed
}

/// The Transform effect (Effect › Distort & Transform › Transform…).
const TRANSFORM: &str = "distort.transform";

/// The Transform effect's dialog: Transform Each's Scale and Move sliders and Rotate dial, then
/// Copies, the reflections, the reference point and Random. Returns whether the values differ from
/// those last previewed.
fn transform_fields(ui: &mut egui::Ui, d: &mut Dialog, unit: vectorcraft_doc::Unit) -> bool {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| transform_each::sections(ui, d, unit));
        ui.add_space(16.0);
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Options"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                widgets::dim_label(ui, tl!("Copies:"));
                if let Some(n) = widgets::range_field(ui, "fx-copies", d.f64("copies", 0.0), 0.0..=1000.0, "", 0, 50.0) {
                    d.fields.insert("copies".into(), json!(n));
                }
            });
            transform_each::reflect_options(ui, d);
        });
    });
    values_changed(d)
}

/// Curves' graph of its `points` (input across, output up, 0..255): drag a point to move it
/// between its neighbours, press on the graph away from the points to add one there, drag one
/// off the graph to remove it (two always stay).
fn curve_graph(ui: &mut egui::Ui, d: &mut Dialog) {
    const SIZE: f32 = 200.0;
    /// How near (screen points) a press must be to take a point.
    const GRAB: f32 = 8.0;
    let t = Tokens::get(ui.ctx());
    let mut pts = vectorcraft_effects::curve_points(d.fields.get("points"));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), egui::Sense::click_and_drag());
    let screen = |(x, y): (f32, f32)| egui::pos2(rect.left() + x * SIZE, rect.bottom() - y * SIZE);
    let unit = |p: egui::Pos2| (((p.x - rect.left()) / SIZE).clamp(0.0, 1.0), ((rect.bottom() - p.y) / SIZE).clamp(0.0, 1.0));
    let held = ui.id().with("curve-point");
    let mut grabbed: Option<usize> = ui.data(|m| m.get_temp(held)).flatten();
    let mut changed = false;
    if let Some(p) = resp.interact_pointer_pos()
        && resp.drag_started()
    {
        let near =
            pts.iter().enumerate().map(|(i, q)| (i, screen(*q).distance(p))).filter(|(_, dist)| *dist <= GRAB).min_by(|a, b| a.1.total_cmp(&b.1));
        grabbed = match near {
            Some((i, _)) => Some(i),
            None => {
                let q = unit(p);
                let i = pts.partition_point(|a| a.0 < q.0);
                pts.insert(i, q);
                changed = true;
                Some(i)
            }
        };
    }
    if let (Some(i), Some(p)) = (grabbed, resp.interact_pointer_pos())
        && resp.dragged()
    {
        if !rect.expand(24.0).contains(p) && pts.len() > 2 {
            pts.remove(i.min(pts.len() - 1));
            grabbed = None;
        } else {
            let step = 1.0 / 255.0;
            let lo = if i == 0 { 0.0 } else { pts.get(i - 1).map_or(0.0, |a| a.0 + step) };
            let hi = pts.get(i + 1).map_or(1.0, |a| a.0 - step);
            let (x, y) = unit(p);
            if let Some(q) = pts.get_mut(i) {
                *q = (x.clamp(lo, hi.max(lo)), y);
            }
        }
        changed = true;
    }
    if resp.drag_stopped() {
        grabbed = None;
    }
    ui.data_mut(|m| m.insert_temp(held, grabbed));
    // The graph: quarter grid, the identity line, the curve and its points.
    let painter = ui.painter_at(rect.expand(4.0));
    painter.rect_filled(rect, 2.0, t.input);
    let faint = egui::Stroke::new(1.0, t.input_border);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        painter.line_segment([screen((f, 0.0)), screen((f, 1.0))], faint);
        painter.line_segment([screen((0.0, f)), screen((1.0, f))], faint);
    }
    painter.line_segment([screen((0.0, 0.0)), screen((1.0, 1.0))], faint);
    let curve: Vec<egui::Pos2> = (0..=64).map(|k| k as f32 / 64.0).map(|x| screen((x, vectorcraft_effects::curve_at(&pts, x)))).collect();
    painter.add(egui::Shape::line(curve, egui::Stroke::new(1.5, t.text)));
    for (i, q) in pts.iter().enumerate() {
        let fill = if Some(i) == grabbed { t.accent } else { t.panel };
        painter.circle(screen(*q), 4.0, fill, egui::Stroke::new(1.5, t.text));
    }
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
    if changed {
        let text: Vec<String> = pts.iter().map(|(x, y)| format!("{},{}", (x * 255.0).round(), (y * 255.0).round())).collect();
        d.fields.insert("points".into(), json!(text.join(" ")));
    }
}

/// What OK runs: `effect.setParams` on the edited effect (`__index`), else `effect.apply` of the
/// dialog's effect with its values on the target item.
fn command(d: &Dialog) -> (&'static str, Value) {
    let params = form::params(d);
    let item = d.fields.get("__item").cloned();
    if let Some(index) = d.fields.get("__index") {
        return ("effect.setParams", json!({"index": index, "item": item.unwrap_or(Value::Null), "params": params}));
    }
    let mut p = json!({"effect": d.str("__effect"), "params": params});
    if let Some(item) = item {
        p["item"] = item;
    }
    ("effect.apply", p)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (cmd, p) = command(d);
    let r = if app.session.in_interaction() {
        // Live preview already applied: keep it (the interaction commits as one undo step).
        let _ = app.session.preview(cmd, &p);
        app.session.commit_interaction().map(|_| Value::Null).map_err(|e| e.to_string())
    } else {
        app.run(cmd, p.clone())
    };
    // Last Effect repeats applied effects, not edits.
    if cmd == "effect.apply" {
        app.last_effect = Some((d.str("__effect"), p["params"].clone()));
    }
    app.ui.dialog = None;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_that_pick_a_value_are_dropdowns_of_its_values() {
        // The values come from the effect's parameter documentation (`joins: "miter"|"round"|…`).
        for (effect, key) in
            [("path.offsetPath", "joins"), ("distort.zigZag", "points"), ("distort.roughen", "points"), ("stylize.innerGlow", "source")]
        {
            let doc = vectorcraft_effects::effect_info(effect).unwrap().params;
            let listed = doc.split(&format!("{key}: ")).nth(1).unwrap().split([',', '}', ' ']).next().unwrap().to_string();
            let values: Vec<String> = choices(effect, key).unwrap().iter().map(|(_, v)| format!("\"{v}\"")).collect();
            assert_eq!(values.join("|"), listed, "{effect} {key}");
        }
    }
}
