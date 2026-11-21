//! Duotone's dialog (Effect › Color Effects › Duotone…): a row per tone (on or off, its swatch or
//! process colour), the curve of the tone picked under Curve (the graph, its points as text and
//! the Reset buttons) and Skip Objects with Spot Colors.
//!
//! Fields: `tones` (the effect's, always [`MAX_TONES`] while the dialog is open, the ones after
//! the first off), `skipSpot`, and `__tone`, the tone whose curve shows.

use craft::theme::Rgba;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, RowAlign, Show as _, ShowContainer as _};
use craft::ui::display::Picture;
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::ThumbAspect;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::text::HexField;
use egui::{Response, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_color::Color;
use vectorcraft_effects::addons::duotone::{MAX_TONES, tone_curve_points};

use super::effect_adjust::{curve_body, json_value};
// PLAN: assumed `EffectCx` is declared in `effect_params.rs` (ef.C3, effects-w4).
use super::effect_params::EffectCx;
use crate::panels::c32;

/// A curve's text: linear, inverse.
const LINEAR: &str = "0,0 100,100";
const INVERSE: &str = "0,100 100,0";

/// The dialog-only field that holds the tone whose curve shows.
const TONE: &str = "__tone";

/// A tone's colour (black when it has none that reads).
fn tone_color(tone: &Value) -> Color {
    match tone.get("color") {
        Some(Value::String(s)) => Color::from_hex(s),
        Some(v @ Value::Object(_)) => serde_json::from_value(v.clone()).ok(),
        _ => None,
    }
    .unwrap_or(Color::BLACK)
}

/// Fills `params["tones"]` up to [`MAX_TONES`] with tones that are off, once, as the dialog opens:
/// the rows write only what the user edits.
pub(super) fn pad_tones(params: &mut Map<String, Value>) {
    let mut tones: Vec<Value> = params.get("tones").and_then(Value::as_array).cloned().unwrap_or_default();
    tones.truncate(MAX_TONES);
    while tones.len() < MAX_TONES {
        tones.push(json!({"color": "#000000", "curve": LINEAR, "active": false}));
    }
    params.insert("tones".into(), Value::Array(tones));
}

/// The names of the tones, in order.
fn tone_labels() -> [&'static str; MAX_TONES] {
    [tl!("Tone 1"), tl!("Tone 2"), tl!("Tone 3"), tl!("Tone 4")]
}

/// The tones in `fields`.
fn tones(fields: &DialogFields) -> Vec<Value> {
    match json_value(fields, "tones") {
        Some(Value::Array(tones)) => tones,
        _ => Vec::new(),
    }
}

/// The rows of the Duotone dialog: the tones, the curve of one of them and the option.
pub(super) fn params(cx: &EffectCx, _fields: &DialogFields) -> Vec<Param> {
    let swatches = cx.swatches.clone();
    vec![
        Param::custom("tones", tl!("Tones"), move |ui, fields, _| tones_row(ui, fields, &swatches)).full_width(),
        Param::section(tl!("Curve")),
        Param::custom(TONE, tl!("Curve"), |ui, fields, _| curve_rows(ui, fields)).companions(["tones"]).full_width(),
        Param::new("skipSpot", tl!("Skip Objects with Spot Colors"), ParamKind::bool(false)),
    ]
}

/// A row per tone; an edit writes the tones back.
fn tones_row(ui: &mut Ui, fields: &mut DialogFields, swatches: &[(String, Color)]) -> Response {
    let mut tones = tones(fields);
    let labels = tone_labels();
    // The first entry is a process colour, the rest are the document's swatches.
    let mut entries = vec![Entry::new(0usize, tl!("Process Color"))];
    entries.extend(swatches.iter().enumerate().map(|(k, (name, _))| Entry::new(k + 1, name.clone())));
    let mut changed = false;
    let shown = Stack::new().fill_width().show(ui, |ui, _| {
        for (i, tone) in tones.iter_mut().enumerate().take(MAX_TONES) {
            changed |= tone_row(ui, (i, labels.get(i).copied().unwrap_or_default()), tone, swatches, &entries);
        }
    });
    if changed {
        fields.set("tones", FieldValue::Json(Value::Array(tones)));
    }
    shown.output
}

/// Tone `i`'s (named `name`) switch, colour chip, swatch (or Process Color) and, as a process colour,
/// its hex value. Returns whether it edited `tone`.
fn tone_row(ui: &mut Ui, (i, name): (usize, &'static str), tone: &mut Value, swatches: &[(String, Color)], entries: &[Entry<usize>]) -> bool {
    let color = tone_color(tone);
    let active = tone.get("active").and_then(Value::as_bool).unwrap_or(true);
    let swatch = tone.get("swatch").and_then(Value::as_str).filter(|s| !s.is_empty());
    let picked = swatch.and_then(|s| swatches.iter().position(|(n, _)| n == s)).map_or(0, |k| k + 1);
    let chip = c32(&color);
    let hex = Rgba::from_hex(&format!("#{}", color.to_hex().trim_start_matches('#'))).unwrap_or(Rgba::BLACK);
    let (mut set_active, mut set_pick, mut set_hex) = (None, None, None);
    Row::new().align(RowAlign::Center).show(ui, |ui, _| {
        if let Some(on) = Checkbox::new(Bind::given(active)).label(name).id_salt(("tone-on", i)).show(ui).value() {
            set_active = Some(*on);
        }
        Picture::painter("", ThumbAspect::SQUARE, move |cx, rect| cx.data_shapes([egui::Shape::rect_filled(rect, 0.0, chip)])).show(ui);
        if let Some(k) = Select::new(name, Bind::given(picked), entries).id_salt(("tone-swatch", i)).show(ui).value() {
            set_pick = Some(*k);
        }
        if picked == 0 {
            Field::new(name).side(HexField::hash_label()).id_salt(("tone-hex", i)).show(ui, |ui, _| {
                if let Some(c) = HexField::hex(name, Bind::given(hex)).id_salt(("tone-hex-digits", i)).show(ui).value() {
                    set_hex = Some(c.to_hex());
                }
            });
        }
    });
    let changed = set_active.is_some() || set_pick.is_some() || set_hex.is_some();
    let Some(t) = tone.as_object_mut() else { return false };
    if let Some(on) = set_active {
        t.insert("active".into(), json!(on));
    }
    if let Some(k) = set_pick {
        match k.checked_sub(1).and_then(|k| swatches.get(k)) {
            Some((name, c)) => {
                t.insert("swatch".into(), json!(name));
                t.insert("color".into(), serde_json::to_value(c).unwrap_or(Value::Null));
            }
            None => {
                t.remove("swatch");
            }
        }
    }
    if let Some(c) = set_hex.and_then(|hex| Color::from_hex(&hex)) {
        t.insert("color".into(), json!(c.to_hex()));
        t.remove("swatch");
    }
    changed
}

/// The picked tone's curve: which tone, the graph with its points as text, and Reset to Linear or to
/// Inverse.
fn curve_rows(ui: &mut Ui, fields: &mut DialogFields) -> Response {
    let picked = (fields.int(TONE).unwrap_or(0).max(0) as usize).min(MAX_TONES - 1);
    let mut tones = tones(fields);
    let pts = tone_curve_points(tones.get(picked).and_then(|t| t.get("curve")));
    let entries: Vec<Entry<usize>> = tone_labels().into_iter().enumerate().map(|(i, label)| Entry::new(i, label)).collect();
    let (mut set_tone, mut set_curve) = (None, None);
    let shown = Stack::new().fill_width().show(ui, |ui, _| {
        Field::new(tl!("Curve")).show(ui, |ui, _| {
            if let Some(i) = Select::new(tl!("Curve"), Bind::given(picked), &entries).id_salt("curve-tone").show(ui).value() {
                set_tone = Some(*i);
            }
        });
        // PLAN: the sheet has Duotone call `curve_row(100, ..)`; that row reads a top-level key and a tone's curve lives inside `tones`, so the shared graph-and-text body is called.
        set_curve = curve_body(ui, 100, &pts).1;
        Row::new().show(ui, |ui, _| {
            for (label, curve) in [(tl!("Reset to Linear"), LINEAR), (tl!("Reset to Inverse"), INVERSE)] {
                if Button::new(label).id_salt(curve).show(ui).clicked() {
                    set_curve = Some(curve.to_owned());
                }
            }
        });
    });
    if let Some(i) = set_tone {
        fields.set(TONE, i as i64);
    }
    if let Some(curve) = set_curve
        && let Some(t) = tones.get_mut(picked).and_then(Value::as_object_mut)
    {
        t.insert("curve".into(), json!(curve));
        fields.set("tones", FieldValue::Json(Value::Array(tones)));
    }
    shown.output
}
