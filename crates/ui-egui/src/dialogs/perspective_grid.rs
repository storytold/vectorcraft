//! View → Perspective Grid → Define Grid…: the grid's type, units and scale, gridline spacing,
//! viewing angle and distance, horizon height and third vanishing point, and the planes' gridline
//! colours and opacity, prefilled from the grid (`perspective.grid.get`). OK runs
//! `perspective.grid.define`: one undo step, nothing when nothing changed.
//!
//! The Preset menu loads a preset's fields (it reads [Custom] once they differ); Save Preset…
//! asks for a name and saves the fields as a preset (`perspective.presets.save`), then comes back.
//! The same dialog is View → Perspective Grid → Save Grid as Preset… and the preset editor of
//! Edit → Perspective Grid Presets (New…, Edit…), whose OK returns to the presets.
//!
//! Fields: those of `perspective.grid.define` (lengths are real-world lengths in `units`; `name`
//! the preset), plus `__mode` (`save`, `edit` or none for Define Grid), `__from` (`define`: Save
//! Preset… returns to Define Grid) and `__original` (the saved preset an edit changes).

use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_tools::distort::perspective::define::is_builtin;
use vectorcraft_tools::distort::perspective::{GridDefinition, Rgb};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Define Perspective Grid.
pub const KIND: &str = "perspectiveGrid";

/// The label column's width.
const LABEL: f32 = 140.0;
/// A value field's width.
const FIELD: f32 = 120.0;

/// The unit the lengths were last shown in.
const SHOWN: &str = "__units";
/// What the dialog is: Define Grid (none), `save` (Save Grid as Preset) or `edit` (the preset
/// editor).
const MODE: &str = "__mode";
const FROM: &str = "__from";
const ORIGINAL: &str = "__original";

/// The Preset menu's entry for fields no preset has.
const CUSTOM: &str = "[Custom]";

/// The lengths among the fields: real-world lengths in `units`.
const LENGTHS: [&str; 3] = ["gridline", "distance", "horizonHeight"];

/// The scale choices: (label, [artboard, real world]).
const SCALES: [(&str, [f64; 2]); 9] = [
    ("1:1", [1.0, 1.0]),
    ("1:2", [1.0, 2.0]),
    ("1:4", [1.0, 4.0]),
    ("1:8", [1.0, 8.0]),
    ("1:12", [1.0, 12.0]),
    ("1:24", [1.0, 24.0]),
    ("1:48", [1.0, 48.0]),
    ("1:96", [1.0, 96.0]),
    ("1:100", [1.0, 100.0]),
];

const TYPES: [&str; 3] = ["One Point Perspective", "Two Point Perspective", "Three Point Perspective"];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| {
        match d.str(MODE).as_str() {
            "save" => tl!("Save Grid as Preset"),
            "edit" => tl!("Perspective Grid Preset"),
            _ => tl!("Define Perspective Grid"),
        }
        .into()
    },
    body,
    confirm,
    min_width: 420.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};

/// Open the dialog on `def`'s fields as `mode` (`""`, `save` or `edit`).
fn open_on(app: &mut VectorcraftApp, mut def: Value, mode: &str) {
    def[SHOWN] = def["units"].clone();
    def[MODE] = json!(mode);
    app.ui.dialog = Some(Dialog::new(KIND, def));
}

/// The active document's grid as Define Grid fields.
fn grid_fields(app: &mut VectorcraftApp) -> Result<Value, String> {
    Ok(app.session.execute("perspective.grid.get", &json!({})).map_err(|e| e.to_string())?["define"].take())
}

/// Open Define Grid on the active document's grid.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = grid_fields(app)?;
    open_on(app, fields, "");
    Ok(Value::Null)
}

/// View → Perspective Grid → Save Grid as Preset…: the grid's fields under a new preset name.
pub fn open_save(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = grid_fields(app)?;
    fields["name"] = json!(app.session.new_perspective_preset_name());
    open_on(app, fields, "save");
    Ok(Value::Null)
}

/// The preset editor: the saved preset `original` (Edit…), or a new preset starting from `base`
/// (New…). Its OK returns to Edit → Perspective Grid Presets.
pub fn open_edit(app: &mut VectorcraftApp, original: Option<&str>, base: &str) -> Result<(), String> {
    let name = original.unwrap_or(base);
    let mut def = json!(app.session.perspective_preset(name).ok_or_else(|| format!("no perspective grid preset named `{name}`"))?);
    match original {
        Some(n) => def[ORIGINAL] = json!(n),
        None => def["name"] = json!(app.session.new_perspective_preset_name()),
    }
    open_on(app, def, "edit");
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = form::params(d);
    match d.str(MODE).as_str() {
        "save" | "edit" => {
            if let Some(original) = d.fields.get(ORIGINAL).and_then(Value::as_str) {
                params["newName"] = params["name"].take();
                params["name"] = json!(original);
            }
            let r = app.run("perspective.presets.save", params)?;
            let name = r["name"].as_str().unwrap_or_default().to_string();
            if d.str(MODE) == "edit" {
                super::perspective_presets::open(app, Some(&name));
            } else if d.str(FROM) == "define" {
                // Back to Define Grid, on the preset just saved.
                if let Some(back) = app.ui.dialog.as_mut() {
                    back.fields.remove(MODE);
                    back.fields.remove(FROM);
                    back.fields.insert("name".into(), json!(name));
                }
            } else {
                app.ui.dialog = None;
            }
            Ok(r)
        }
        _ => {
            let r = app.run("perspective.grid.define", params);
            if r.is_ok() {
                app.ui.dialog = None;
            }
            r
        }
    }
}

/// The unit the lengths are in.
fn unit(d: &Dialog) -> Unit {
    Unit::named(&d.str("units")).unwrap_or_default()
}

fn kind(d: &Dialog) -> u8 {
    d.fields.get("kind").and_then(Value::as_u64).map_or(2, |k| k.clamp(1, 3) as u8)
}

fn scale(d: &Dialog) -> [f64; 2] {
    let v = d.fields.get("scale").and_then(Value::as_array);
    let at = |i: usize| v.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(1.0);
    [at(0), at(1)]
}

/// A length field (`key`, in `unit`) `width` wide. Returns true when it changed.
fn length(ui: &mut egui::Ui, d: &mut Dialog, key: &str, unit: Unit, width: f32) -> bool {
    let pt = d.fields.get(key).and_then(Value::as_f64).map(|v| unit.to_pt(v));
    let Some(v) = widgets::num_field(ui, ("persp-len", key), pt, unit, width) else { return false };
    d.fields.insert(key.into(), json!(unit.from_pt(v)));
    true
}

/// One of the third vanishing point's coordinates (`i`: 0 = x, 1 = y).
fn third_vp(ui: &mut egui::Ui, d: &mut Dialog, i: usize, unit: Unit, enabled: bool) {
    let mut v = d.fields.get("thirdVp").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![json!(0), json!(0)]);
    v.resize(2, json!(0));
    let pt = v.get(i).and_then(Value::as_f64).map(|x| unit.to_pt(x));
    ui.label(if i == 0 { "X:" } else { "Y:" });
    if let Some(x) = ui.add_enabled_ui(enabled, |ui| widgets::num_field(ui, ("persp-vp3", i), pt, unit, 84.0)).inner {
        v[i] = json!(unit.from_pt(x));
        d.fields.insert("thirdVp".into(), Value::Array(v));
    }
}

/// A plane's gridline colour: a chip and its hex.
fn color(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    let t = Tokens::get(ui.ctx());
    let cur = Rgb::parse(&d.str(key));
    widgets::label_row(ui, label, LABEL, |ui| {
        let (chip, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        if let Some(Rgb([r, g, b])) = cur {
            ui.painter().rect_filled(chip, 2.0, egui::Color32::from_rgb(r, g, b));
        }
        ui.painter().rect_stroke(chip, 2.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Inside);
        let hex = cur.map(|c| c.hex()).unwrap_or_default();
        if let Some(c) = widgets::hex_field(ui, ("persp-color", key), hex.trim_start_matches('#')).as_deref().and_then(Rgb::parse) {
            d.fields.insert(key.into(), json!(c.hex()));
        }
    });
}

/// The Name row: in Define Grid the Preset menu (choosing a preset loads its fields) and Save
/// Preset…, else the preset's name.
fn name_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    if !d.str(MODE).is_empty() {
        widgets::label_row(ui, tl!("Name:"), LABEL, |ui| {
            form::text(ui, d, "name", 200.0);
        });
        return;
    }
    let presets = app.session.perspective_presets();
    let name = d.str("name");
    // The fields no longer the named preset's: [Custom].
    let fields: Option<GridDefinition> = serde_json::from_value(form::params(d)).ok();
    let current = presets.iter().find(|p| p.name == name).filter(|p| fields.as_ref().is_some_and(|f| f.same(p)));
    if current.is_none() && !name.is_empty() {
        d.fields.insert("name".into(), json!(""));
    }
    let shown = current.map_or(CUSTOM, |p| p.name.as_str());
    let names: Vec<&str> = std::iter::once(CUSTOM).chain(presets.iter().map(|p| p.name.as_str())).collect();
    // [Custom] and the built-in presets are ours (translated); the saved ones are names.
    let builtin = |k: usize| k == 0 || names.get(k).is_some_and(|n| is_builtin(n));
    widgets::label_row(ui, tl!("Preset:"), LABEL, |ui| {
        let picked = super::mixed_dropdown(ui, "persp-preset", shown, &names, 200.0, builtin);
        if let Some(p) = picked.and_then(|i| i.checked_sub(1)).and_then(|i| presets.get(i)) {
            let mut fields = json!(p);
            fields[SHOWN] = fields["units"].clone();
            if let Some(o) = fields.as_object() {
                d.fields.extend(o.clone());
            }
        }
        if widgets::flat_button(ui, tl!("Save Preset…"), 96.0).on_hover_text(tl!("Save these settings as a preset")).clicked() {
            d.fields.insert(MODE.into(), json!("save"));
            d.fields.insert(FROM.into(), json!("define"));
            d.fields.insert("name".into(), json!(app.session.new_perspective_preset_name()));
        }
    });
    ui.add_space(8.0);
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    name_row(app, ui, d);
    widgets::subheader(ui, tl!("Perspective Grid Settings"));
    ui.add_space(6.0);
    let k = kind(d);
    widgets::label_row(ui, tl!("Type:"), LABEL, |ui| {
        if let Some(i) = widgets::dropdown(ui, "persp-type", TYPES[usize::from(k - 1)], &TYPES, 200.0) {
            d.fields.insert("kind".into(), json!(i + 1));
        }
    });
    let units: Vec<(&str, &str)> = Unit::ALL.iter().filter(|u| **u != Unit::FeetInches).map(|u| (u.key(), u.label())).collect();
    form::choice(ui, d, "units", tl!("Units:"), (LABEL, 200.0), &units);
    // A new unit (the menu or `ui.dialog.set`) keeps the lengths: they are converted.
    let unit = unit(d);
    let before = Unit::named(&d.str(SHOWN)).unwrap_or(unit);
    d.fields.insert(SHOWN.into(), json!(unit.key()));
    if unit != before {
        let f = before.points() / unit.points();
        for key in LENGTHS {
            if let Some(v) = d.fields.get(key).and_then(Value::as_f64) {
                d.fields.insert(key.into(), json!(v * f));
            }
        }
        if let Some(v) = d.fields.get("thirdVp").and_then(Value::as_array) {
            let v: Vec<f64> = v.iter().map(|x| x.as_f64().unwrap_or(0.0) * f).collect();
            d.fields.insert("thirdVp".into(), json!(v));
        }
    }
    let sc = scale(d);
    let named = SCALES.iter().find(|(_, s)| *s == sc).map(|(l, _)| *l);
    let mut labels: Vec<&str> = SCALES.iter().map(|(l, _)| *l).collect();
    labels.push("Custom");
    widgets::label_row(ui, tl!("Scale:"), LABEL, |ui| {
        if let Some(i) = widgets::dropdown(ui, "persp-scale", named.unwrap_or("Custom"), &labels, 200.0) {
            // Custom starts from the scale shown.
            let s = SCALES.get(i).map_or(sc, |(_, s)| *s);
            d.fields.insert("scale".into(), json!(s));
        }
    });
    if named.is_none() {
        widgets::label_row(ui, "", LABEL, |ui| {
            for (i, label) in [tl!("Artboard:"), tl!("Real World:")].into_iter().enumerate() {
                ui.label(egui::RichText::new(label).color(t.text_dim));
                if let Some(v) = widgets::plain_field(ui, ("persp-scale", i), sc[i], "", 3, 56.0).filter(|v| *v > 0.0 && v.is_finite()) {
                    let mut s = sc;
                    s[i] = v;
                    d.fields.insert("scale".into(), json!(s));
                }
            }
        });
    }
    widgets::label_row(ui, tl!("Gridline every:"), LABEL, |ui| {
        length(ui, d, "gridline", unit, FIELD);
    });
    if sc[0] != sc[1] {
        let real = unit.number(unit.to_pt(sc[1] / sc[0]));
        let note = crate::i18n::fmt(
            tl!("1 {unit} on the artboard stands for {real} {unit} in the scene."),
            &[("unit", unit.suffix()), ("real", &real.to_string())],
        );
        widgets::label_row(ui, "", LABEL, |ui| {
            ui.label(egui::RichText::new(note).size(11.5).color(t.text_dim));
        });
    }
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Viewing Angle:"), LABEL, |ui| {
        let a = d.f64("angle", 45.0);
        if let Some(v) = ui.add_enabled_ui(k != 1, |ui| widgets::plain_field(ui, "persp-angle", a, "°", 2, FIELD)).inner {
            d.fields.insert("angle".into(), json!(v));
        }
    });
    widgets::label_row(ui, tl!("Viewing Distance:"), LABEL, |ui| {
        length(ui, d, "distance", unit, FIELD);
    });
    widgets::label_row(ui, tl!("Horizon Height:"), LABEL, |ui| {
        length(ui, d, "horizonHeight", unit, FIELD);
    });
    widgets::label_row(ui, tl!("Third Vanishing Point:"), LABEL, |ui| {
        third_vp(ui, d, 0, unit, k == 3);
        third_vp(ui, d, 1, unit, k == 3);
    });
    ui.add_space(12.0);
    widgets::subheader(ui, tl!("Grid Color & Opacity"));
    ui.add_space(6.0);
    color(ui, d, "leftColor", tl!("Left Grid:"));
    color(ui, d, "rightColor", tl!("Right Grid:"));
    color(ui, d, "groundColor", tl!("Horizontal Grid:"));
    // Opacity: a slider and its field, in the label column the rows above use.
    widgets::label_row(ui, tl!("Opacity:"), LABEL, |ui| {
        let v = d.f64("opacity", 50.0).clamp(0.0, 100.0);
        let track = |x: f32| egui::Color32::from_gray((60.0 + x * 160.0) as u8);
        let shown = widgets::SliderValue::field(v, 0.0..=100.0, "%", 0);
        let mut new = widgets::color_slider_valued(ui, "persp-opacity", tl!("Opacity:"), Some(shown), (v / 100.0) as f32, form::SLIDER_WIDTH, &track)
            .0
            .map(|x| (f64::from(x) * 100.0).round());
        if let Some(x) = widgets::plain_field(ui, "persp-opacity-field", v, "%", 0, 52.0) {
            new = Some(x.round().clamp(0.0, 100.0));
        }
        if let Some(n) = new.filter(|n| *n != v) {
            d.fields.insert("opacity".into(), json!(n));
        }
    });
    false
}
