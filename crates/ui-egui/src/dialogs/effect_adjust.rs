//! The rows of the colour adjustment dialogs (Effect › Color Adjustments) and the curve row Curves
//! and Duotone share: a slider table with colour rails, the channel, Curves' graph with its points
//! as text, the target colour, Levels' gamma and the options.

use craft::imaging::ui::adjust::{AdjustField, AdjustSliders, AdjustTrack};
use craft::imaging::ui::{Curve, CurveEditor};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamKind};
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use craft::ui::text::TextField;
use egui::{Color32, Response, Ui};
use serde_json::Value;
use vectorcraft_color::Color;
use vectorcraft_effects::addons::duotone::tone_curve_points;
use vectorcraft_effects::{curve_at, curve_points, default_params};

// PLAN: assumed `EffectCx` is declared in `effect_params.rs` (ef.C3, effects-w4).
use super::effect_params::{EffectCx, own_choices};
use crate::panels::c32;

/// A slider of a colour adjustment dialog: (effect, field, label, range, rail colour at 0..1).
type Slider = (&'static str, &'static str, &'static str, (f64, f64), fn(f32) -> Color32);

/// How many colours sample a rail: enough that the track reads as the rail's gradient.
const RAIL_STOPS: usize = 13;

fn rgb(r: f32, g: f32, b: f32) -> Color32 {
    c32(&Color::rgb(r, g, b))
}

fn grey_rail(x: f32) -> Color32 {
    rgb(x, x, x)
}

fn hue_rail(x: f32) -> Color32 {
    c32(&Color::from_hsb(((x - 0.5) * 360.0).rem_euclid(360.0), 0.8, 0.9))
}

fn saturation_rail(x: f32) -> Color32 {
    c32(&Color::from_hsb(0.0, x, 0.9))
}

fn temperature_rail(x: f32) -> Color32 {
    rgb(0.35 + 0.65 * x, 0.55 + 0.05 * x, 1.0 - 0.8 * x)
}

fn tint_rail(x: f32) -> Color32 {
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

/// The value under `key` as JSON, in whatever form the dialog stored it: the one reader of the rows
/// that keep structured values (tones, corners, curve points).
pub(super) fn json_value(fields: &DialogFields, key: &str) -> Option<Value> {
    serde_json::to_value(fields.get(key)?).ok()
}

/// The rows of adjustment `cx.id`: its slider table, then the channel, the curve, the target colour,
/// the gamma and the options, each when the effect's values have the key.
pub(super) fn params(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let effect = cx.id.as_str();
    let defaults = default_params(effect).unwrap_or_default();
    let table: Vec<AdjustField> = SLIDERS
        .iter()
        .filter(|s| s.0 == effect)
        .map(|&(_, key, label, (min, max), rail)| {
            let stops = (0..RAIL_STOPS).map(|k| DataColor::new(rail(k as f32 / (RAIL_STOPS - 1) as f32))).collect();
            AdjustField::new(key, tl!(label), min..=max, defaults.get(key).and_then(Value::as_f64).unwrap_or(min)).track(AdjustTrack::Colors(stops))
        })
        .collect();
    let mut rows = Vec::new();
    if !table.is_empty() {
        let keys: Vec<&'static str> = table.iter().map(|f| f.key).collect();
        rows.push(sliders_row(table).companions(keys));
    }
    if fields.contains("channel") {
        let options = own_choices(effect, "channel", false)
            .unwrap_or_default()
            .into_iter()
            .map(|(label, value)| (value.to_owned(), Label::from(label)))
            .collect();
        rows.push(Param::new("channel", tl!("Channel"), ParamKind::choice(options, None)));
    }
    if fields.contains("points") {
        rows.push(curve_row(255, "points"));
    }
    if fields.contains("color") {
        rows.push(Param::new("color", tl!("Color"), ParamKind::color(None)));
    }
    if fields.contains("gamma") {
        rows.push(Param::new("gamma", tl!("Gamma"), ParamKind::number(Some(0.1), Some(10.0), 1.0, 0.01, 2)));
    }
    for (key, label) in [("colorize", tl!("Colorize")), ("preserveLightness", tl!("Preserve Lightness"))] {
        if fields.contains(key) {
            rows.push(Param::new(key, label, ParamKind::bool(false)));
        }
    }
    rows
}

/// The slider table as one row over the `table`'s keys, which it reads and writes as numbers.
fn sliders_row(table: Vec<AdjustField>) -> Param {
    Param::custom("sliders", tl!("Adjustments"), move |ui, fields, _| {
        let values: Vec<f64> = table.iter().map(|f| fields.number(f.key).unwrap_or(f.default)).collect();
        let edit = AdjustSliders::new(tl!("Adjustments"), &table, Bind::given(values)).show(ui);
        if let Some(now) = edit.events.landed() {
            for (field, value) in table.iter().zip(now) {
                fields.set(field.key, *value);
            }
        }
        edit.response.clone()
    })
    .full_width()
}

/// The points of a curve of `levels` levels (0..1 each) from its stored value: the effect's own
/// reader of that scale.
// PLAN: the effects crate exports no reader of a given scale (`curve_points_on` is private to it); the two scales the dialogs use pick their exported reader.
fn curve_points_of(levels: u32, value: Option<&Value>) -> Vec<(f32, f32)> {
    if levels == 100 { tone_curve_points(value) } else { curve_points(value) }
}

/// Curve points in level units as the `"x,y x,y …"` text of the effect's `points` and `curve` parameters.
fn points_text(pts: impl IntoIterator<Item = [f32; 2]>) -> String {
    pts.into_iter().map(|[x, y]| format!("{},{}", x.round(), y.round())).collect::<Vec<_>>().join(" ")
}

/// A curve row over the points text under `key`, on a scale of 0..`levels`: the graph and the points
/// as text. Curves' (255) and, through [`curve_body`], each Duotone tone's (100).
pub(super) fn curve_row(levels: u32, key: &str) -> Param {
    let key = key.to_owned();
    let at = key.clone();
    Param::custom(key, tl!("Curve"), move |ui, fields, _| {
        let pts = curve_points_of(levels, json_value(fields, &at).as_ref());
        let (response, text) = curve_body(ui, levels, &pts);
        if let Some(text) = text {
            fields.set(at.as_str(), FieldValue::Text(text));
        }
        response
    })
    .full_width()
}

/// The graph of the curve through `pts` (0..1 each; input across, output up) on a scale of 0..`levels`
/// and its points as text. Returns the body's response and the new points text when the graph or
/// the text was edited. The graph draws the curve the effect applies.
pub(super) fn curve_body(ui: &mut Ui, levels: u32, pts: &[(f32, f32)]) -> (Response, Option<String>) {
    let scale = levels.max(1) as f32;
    let curve = Curve::smooth(pts.iter().map(|&(x, y)| [(x * scale).round(), (y * scale).round()]).collect());
    let text = points_text(pts.iter().map(|&(x, y)| [x * scale, y * scale]));
    let mut edited = None;
    let shown = Stack::new().fill_width().show(ui, |ui, _| {
        // PLAN: the sheet names `CurveSampler::new`; the editor's `sampler` prop takes the closure itself.
        let graph = CurveEditor::new(tl!("Curve"), Bind::given(curve.clone()))
            .levels(levels)
            .bars(false)
            .fields(false)
            .sampler(move |points: &[[f32; 2]]| {
                let unit: Vec<(f32, f32)> = points.iter().map(|&[x, y]| (x / scale, y / scale)).collect();
                (0..=levels).map(|k| curve_at(&unit, k as f32 / scale) * scale).collect()
            })
            .show(ui);
        if let Some(points) = graph.events.landed().and_then(Curve::points) {
            edited = Some(points_text(points.iter().copied()));
        }
        Field::new(tl!("Points")).show(ui, |ui, _| {
            if let Some(typed) = TextField::new(tl!("Points"), Bind::given(text.clone())).id_salt("points").show(ui).events.landed() {
                edited = Some(typed.clone());
            }
        });
    });
    (shown.output, edited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::testing::Fixture;
    use egui::{Key, Modifiers};
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn graph_escape_restores_the_curve_during_a_held_drag() {
        for levels in [255, 100] {
            let original = format!("0,0 {},{} {levels},{levels}", levels / 2, levels / 2);
            let stored = Rc::new(RefCell::new(original.clone()));
            let shown = Rc::clone(&stored);
            let mut fixture = Fixture::new(move |ui| {
                let value = Value::String(shown.borrow().clone());
                let pts = curve_points_of(levels, Some(&value));
                if let Some(text) = curve_body(ui, levels, &pts).1 {
                    *shown.borrow_mut() = text;
                }
            });
            fixture.room(egui::vec2(480.0, 600.0)).settle();
            let from = fixture.by_label("Point 2 of 3").expect("the interior curve point").center();
            let by = egui::vec2(0.0, -30.0);
            fixture.drag_hold_at(from, by).settle();
            assert_ne!(*stored.borrow(), original, "a held graph drag previews its new points");
            fixture.key(Key::Escape, Modifiers::NONE).settle();
            fixture.release_at(from + by).settle();
            assert_eq!(*stored.borrow(), original, "Escape before release restores the opening curve");

            let from = fixture.by_label("Point 2 of 3").unwrap().center();
            fixture.drag_hold_at(from, by).settle();
            let committed = stored.borrow().clone();
            assert_ne!(committed, original);
            fixture.release_at(from + by).settle();
            assert_eq!(*stored.borrow(), committed, "releasing the drag keeps the new curve");
            fixture.key(Key::Escape, Modifiers::NONE).settle();
            assert_eq!(*stored.borrow(), committed, "Escape after release leaves the committed curve");
        }
    }

    #[test]
    fn brightness_slider_escape_restores_hosted_fields_and_the_live_effect() {
        use crate::tests::panel_harness::Panel;
        use craft::ui::contract::Role;
        use serde_json::json;
        use vectorcraft_doc::NodeId;

        fn draw(app: &mut crate::VectorcraftApp, ui: &mut Ui) {
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| host.run(ui));
            }
            super::super::host::pump(app, ui.ctx());
        }

        fn brightness(app: &crate::VectorcraftApp, id: NodeId) -> f64 {
            app.session
                .active()
                .unwrap()
                .doc
                .node(id)
                .unwrap()
                .appearance
                .effects
                .iter()
                .find(|effect| effect.id == "adjust.brightnessContrast")
                .unwrap()
                .params["brightness"]
                .as_f64()
                .unwrap()
        }

        let mut app = crate::testing::windowed(crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let id = NodeId(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 60, "height": 40})).unwrap()["id"].as_u64().unwrap());
        let original = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let mut panel = Panel::new(app, draw);
        panel.room(egui::vec2(800.0, 600.0));
        panel.app().run("effect.dialog", json!({"effect": "adjust.brightnessContrast", "item": null})).unwrap();
        panel.frame();
        let dialog = panel.app().effect_dialogs.effect.as_ref().unwrap().id;
        assert_eq!(brightness(&panel.app(), id), 0.0, "the default live effect starts at zero brightness");
        panel.drag_hold_role("Brightness", Role::SLIDER, egui::vec2(35.0, 0.0));
        let proposed = brightness(&panel.app(), id);
        assert_ne!(proposed, 0.0, "dragging updates the live preview");
        assert_eq!(super::super::tests_warp::json_fields(&panel.app(), dialog)["brightness"], proposed);
        panel.key(Key::Escape, Modifiers::NONE);
        panel.release();
        assert_eq!(super::super::tests_warp::json_fields(&panel.app(), dialog)["brightness"], 0.0);
        assert_eq!(brightness(&panel.app(), id), 0.0, "Escape restores the previous preview");
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);

        panel.click_role("Brightness", Role::SPIN_BUTTON);
        panel.key(Key::A, Modifiers::COMMAND);
        panel.type_text("25");
        panel.key(Key::Enter, Modifiers::NONE);
        assert!(panel.app().effect_dialogs.effect.is_none(), "Enter confirms the valid brightness and closes the dialog");
        assert_eq!(brightness(&panel.app(), id), 25.0);
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
        panel.app().run("edit.undo", json!({})).unwrap();
        assert_eq!(panel.app().session.active().unwrap().doc, original);
    }

    #[test]
    fn points_escape_restores_the_curve_after_live_typing() {
        // The same body edits Curves in 255 levels and each Duotone tone in 100 levels.
        for levels in [255, 100] {
            let original = format!("0,0 {levels},{levels}");
            let changed = format!("0,0 {},{levels} {levels},{levels}", levels / 2);
            let stored = Rc::new(RefCell::new(original.clone()));
            let shown = Rc::clone(&stored);
            let mut fixture = Fixture::new(move |ui| {
                let value = Value::String(shown.borrow().clone());
                let pts = curve_points_of(levels, Some(&value));
                if let Some(text) = curve_body(ui, levels, &pts).1 {
                    *shown.borrow_mut() = text;
                }
            });
            fixture.room(egui::vec2(480.0, 600.0)).settle();
            let points = fixture.by_label("Points").expect("curve Points field");
            fixture.click(&points).key(Key::A, Modifiers::COMMAND).type_text(&changed).settle();
            assert_eq!(*stored.borrow(), changed, "typing updates the live curve");
            fixture.key(Key::Escape, Modifiers::NONE).settle();
            assert_eq!(*stored.borrow(), original, "Escape restores the curve before typing began");

            let points = fixture.by_label("Points").unwrap();
            fixture.click(&points).key(Key::A, Modifiers::COMMAND).type_text(&changed).key(Key::Enter, Modifiers::NONE).settle();
            assert_eq!(*stored.borrow(), changed, "a committed edit keeps the new curve");
            fixture.key(Key::Escape, Modifiers::NONE).settle();
            assert_eq!(*stored.borrow(), changed, "Escape after commit cannot undo the accepted curve");
        }
    }
}
