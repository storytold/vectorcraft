//! Blend panel: Make, Release, Expand, Reverse Spine and Replace Spine, and the selected blend's
//! options (spacing, orientation, step easing and colour easing), applied as they change so the
//! canvas shows them at once. With no blend selected the options are those new blends start with.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// Spacing modes: (value, label).
const SPACING: [(&str, &str); 3] = [("smooth", "Smooth Color"), ("steps", "Specified Steps"), ("distance", "Specified Distance")];

/// Easing curves: (value, label).
const EASING: [(&str, &str); 4] = [("linear", "Linear"), ("easeIn", "Ease In"), ("easeOut", "Ease Out"), ("easeInOut", "Ease In and Out")];

/// Width of the label column.
const LABEL_W: f32 = 74.0;

fn info(app: &mut VectorcraftApp) -> Value {
    app.session.execute("object.blend.info", &json!({})).unwrap_or(Value::Null)
}

fn set(app: &mut VectorcraftApp, params: Value) {
    if let Err(e) = app.run("object.blend.options", params) {
        app.status(e);
    }
}

fn action(app: &mut VectorcraftApp, ui: &mut Ui, label: &str, id: &str) {
    let on = crate::menus::enabled(app, id);
    if ui.add_enabled(on, egui::Button::new(tl!(label))).clicked()
        && let Err(e) = app.run(id, json!({}))
    {
        app.status(e);
    }
}

/// A dropdown of `options` (value, label) showing `current`; the chosen value.
fn choose(ui: &mut Ui, id: &str, options: &[(&'static str, &'static str)], current: &str) -> Option<&'static str> {
    let labels: Vec<&str> = options.iter().map(|&(_, l)| tl!(l)).collect();
    let shown = options.iter().position(|(v, _)| *v == current).and_then(|i| labels.get(i).copied()).unwrap_or_default();
    widgets::dropdown(ui, id, shown, &labels, 132.0).and_then(|i| options.get(i)).map(|(v, _)| *v).filter(|v| *v != current)
}

/// A strength field (0..100 %).
fn strength(ui: &mut Ui, id: &str, value: f64, enabled: bool) -> Option<f64> {
    ui.add_enabled_ui(enabled, |ui| widgets::spin_plain(ui, id, value.round(), "%", 0, 72.0, 5.0, 0.0, &[0.0, 25.0, 50.0, 75.0, 100.0]))
        .inner
        .map(|v| v.clamp(0.0, 100.0))
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let v = info(app);
    let is_blend = v["target"] == "blend";
    // The commands.
    ui.horizontal_wrapped(|ui| {
        action(app, ui, "Make", "object.blend.make");
        action(app, ui, "Release", "object.blend.release");
        action(app, ui, "Expand", "object.blend.expand");
    });
    ui.horizontal_wrapped(|ui| {
        action(app, ui, "Reverse Spine", "object.blend.reverseSpine");
        action(app, ui, "Replace Spine", "object.blend.replaceSpine");
    });
    ui.add_space(6.0);
    widgets::subheader(ui, if is_blend { tl!("Selected blend") } else { tl!("New blends") });
    let spacing = v["spacing"].as_str().unwrap_or("smooth").to_string();
    widgets::label_row(ui, tl!("Spacing:"), LABEL_W, |ui| {
        if let Some(s) = choose(ui, "blend-spacing", &SPACING, &spacing) {
            set(app, json!({ "spacing": s }));
        }
    });
    match spacing.as_str() {
        "steps" => widgets::label_row(ui, tl!("Steps:"), LABEL_W, |ui| {
            let n = v["steps"].as_f64().unwrap_or(5.0);
            if let Some(n) = widgets::spin_plain(ui, "blend-steps", n, "", 0, 72.0, 1.0, 1.0, &[1.0, 5.0, 10.0, 25.0, 50.0, 100.0]) {
                set(app, json!({ "spacing": "steps", "steps": n.round().clamp(1.0, 1000.0) }));
            }
        }),
        "distance" => widgets::label_row(ui, tl!("Distance:"), LABEL_W, |ui| {
            let d = v["distance"].as_f64().unwrap_or(10.0);
            if let Some(d) = widgets::spin_plain(ui, "blend-distance", d, " pt", 2, 72.0, 1.0, 0.1, &[1.0, 5.0, 10.0, 25.0, 50.0]) {
                set(app, json!({ "spacing": "distance", "distance": d }));
            }
        }),
        _ => {}
    }
    widgets::label_row(ui, tl!("Orientation:"), LABEL_W, |ui| {
        let path = v["orientation"] == "path";
        if widgets::radio(ui, tl!("Page"), !path, true) && path {
            set(app, json!({ "orientation": "page" }));
        }
        if widgets::radio(ui, tl!("Path"), path, true) && !path {
            set(app, json!({ "orientation": "path" }));
        }
    });
    // Easing applies to the selected blend (new blends start even).
    ui.add_space(4.0);
    let easing = v["easing"].as_str().unwrap_or("linear").to_string();
    ui.add_enabled_ui(is_blend, |ui| {
        widgets::label_row(ui, tl!("Steps ease:"), LABEL_W, |ui| {
            if let Some(e) = choose(ui, "blend-easing", &EASING, &easing) {
                set(app, json!({ "easing": e }));
            }
        });
        widgets::label_row(ui, tl!("Strength:"), LABEL_W, |ui| {
            if let Some(s) = strength(ui, "blend-strength", v["strength"].as_f64().unwrap_or(50.0), easing != "linear") {
                set(app, json!({ "strength": s }));
            }
        });
        let color = v["colorEasing"].as_str().unwrap_or("same").to_string();
        let mut color_options = vec![("same", "Same as Steps")];
        color_options.extend(EASING);
        widgets::label_row(ui, tl!("Color ease:"), LABEL_W, |ui| {
            if let Some(e) = choose(ui, "blend-color-easing", &color_options, &color) {
                set(app, json!({ "colorEasing": e }));
            }
        });
        widgets::label_row(ui, tl!("Strength:"), LABEL_W, |ui| {
            let on = color != "same" && color != "linear";
            if let Some(s) = strength(ui, "blend-color-strength", v["colorStrength"].as_f64().unwrap_or(50.0), on) {
                set(app, json!({ "colorStrength": s }));
            }
        });
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    for (label, id) in [
        ("Make", "object.blend.make"),
        ("Release", "object.blend.release"),
        ("Expand", "object.blend.expand"),
        ("Replace Spine", "object.blend.replaceSpine"),
        ("Reverse Spine", "object.blend.reverseSpine"),
        ("Reverse Front to Back", "object.blend.reverseFrontToBack"),
    ] {
        if menu_item(ui, tl!(label), crate::menus::enabled(app, id), false)
            && let Err(e) = app.run(id, json!({}))
        {
            app.status(e);
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Blend Options…"), true, false) {
        crate::menus::invoke(app, "object.blend.options", Value::Null);
    }
}
