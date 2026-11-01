//! Blend panel (Window → Blend): Make, the Spacing and Orientation of the selected blend (with no
//! blend selected, the options new blends start with), Reverse Front to Back, Reverse Spine,
//! Replace Spine, Expand and Release. Each control runs its Object → Blend command
//! (`object.blend.make`, `object.blend.options` and the others), so a change of the selected
//! blend's spacing or orientation shows on the canvas at once as one undo step. Make is dimmed
//! until two or more objects are selected. Spacing and Orientation are dimmed until a blend or two
//! or more objects are selected, and the other buttons are dimmed until a blend is selected.

use egui::Ui;
use serde_json::{Value, json};

use super::selection_len;
use crate::VectorcraftApp;
use crate::dialogs::blend_options::{SPACING, orientation_button};
use crate::widgets;

pub const ID: &str = "blend";

/// Width of the label column, as in Blend Options.
const LABEL_W: f32 = 82.0;

/// Width of the Specified Steps and Specified Distance field.
const FIELD_W: f32 = 72.0;

/// A row of flat buttons sharing the row's width, each (label, command): a click runs the command,
/// and a button whose command is unavailable is dimmed.
fn buttons(app: &mut VectorcraftApp, ui: &mut Ui, row: &[(&str, &str)]) {
    let gap = ui.spacing().item_spacing.x;
    let n = row.len() as f32;
    let width = ((ui.available_width() - gap * (n - 1.0)) / n).max(48.0);
    ui.horizontal(|ui| {
        for &(label, cmd) in row {
            let on = available(app, cmd);
            if ui.add_enabled_ui(on, |ui| widgets::flat_button(ui, label, width)).inner.clicked() {
                app.run(cmd, json!({})).ok();
            }
        }
    });
}

/// Can `cmd` run on the selection? Make needs two or more objects, and the other commands need a
/// selected blend.
fn available(app: &VectorcraftApp, cmd: &str) -> bool {
    crate::menus::enabled(app, cmd) && (cmd != "object.blend.make" || selection_len(app) >= 2)
}

/// `object.blend.options` parameters for spacing mode `mode`, with the step count and distance
/// the panel shows.
fn spacing_params(mode: &str, steps: f64, distance: f64) -> Value {
    match mode {
        "steps" => json!({"spacing": "steps", "steps": steps}),
        "distance" => json!({"spacing": "distance", "distance": distance}),
        _ => json!({"spacing": "smooth"}),
    }
}

/// Spacing and Orientation, as `object.blend.info` reports them; a change runs
/// `object.blend.options`.
fn settings(app: &mut VectorcraftApp, ui: &mut Ui) {
    let info = app.session.execute("object.blend.info", &json!({})).unwrap_or_default();
    let unit = app.session.general_unit();
    let mode = info["spacing"].as_str().unwrap_or("smooth");
    let steps = info["steps"].as_f64().unwrap_or(5.0);
    let distance = info["distance"].as_f64().unwrap_or(10.0);
    let path = info["orientation"] == "path";
    let mut change = None;
    let shown = SPACING.iter().find(|(v, _)| *v == mode).map_or(SPACING[0].1, |(_, l)| l);
    let labels = SPACING.map(|(_, l)| l);
    widgets::label_row(ui, tl!("Spacing:"), LABEL_W, |ui| {
        let width = ui.available_width().max(FIELD_W);
        if let Some((value, _)) = widgets::dropdown(ui, "blend-panel-spacing", shown, &labels, width).and_then(|i| SPACING.get(i)) {
            change = Some(spacing_params(value, steps, distance));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W);
        if mode == "distance" {
            if let Some(d) = widgets::num_field(ui, "blend-panel-distance", Some(distance), unit, FIELD_W) {
                change = Some(spacing_params("distance", steps, d));
            }
        } else {
            // Smooth Color picks the count itself: the field shows the steps, disabled.
            ui.add_enabled_ui(mode == "steps", |ui| {
                if let Some(n) = widgets::range_field(ui, "blend-panel-steps", steps, 1.0..=1000.0, "", 0, FIELD_W) {
                    change = Some(spacing_params("steps", n, distance));
                }
            });
        }
    });
    widgets::label_row(ui, tl!("Orientation:"), LABEL_W, |ui| {
        for (value, along, tip) in [("page", false, tl!("Align to Page")), ("path", true, tl!("Align to Path"))] {
            if orientation_button(ui, along, path == along, tip) {
                change = Some(json!({ "orientation": value }));
            }
        }
    });
    let selected = info["target"] == "blend";
    // Position easing needs a spine to run along: a replaced one, or Align to Path.
    let spined = info["spine"]["explicit"] == true || info["orientation"] == "path";
    ui.add_enabled_ui(selected && spined, |ui| {
        let modes = ["easeIn", "easeOut", "easeInOut", "easeOutIn"];
        let labels = [tl!("Ease In"), tl!("Ease Out"), tl!("Ease In Out"), tl!("Ease Out In")];
        let mode = info["easingType"].as_str().unwrap_or(modes[0]);
        let index = modes.iter().position(|m| *m == mode).unwrap_or(0);
        widgets::label_row(ui, tl!("Type:"), LABEL_W, |ui| {
            if let Some(i) = widgets::dropdown(ui, "blend-panel-easing", labels[index], &labels, ui.available_width().max(FIELD_W))
                && let Some(mode) = modes.get(i)
            {
                change = Some(json!({"easingType": mode}));
            }
        });
        widgets::label_row(ui, tl!("Ramp:"), LABEL_W, |ui| {
            let mut ramp = info["ramp"].as_f64().unwrap_or(0.0);
            if ui.add(egui::Slider::new(&mut ramp, 0.0..=100.0).suffix("%")).changed() {
                change = Some(json!({"ramp": ramp}));
            }
        });
    });
    ui.add_enabled_ui(selected, |ui| {
        widgets::label_row(ui, tl!("Shift:"), LABEL_W, |ui| {
            let mut shift = info["colorShift"].as_f64().unwrap_or(0.0);
            if ui.add(egui::Slider::new(&mut shift, -100.0..=100.0).suffix("%")).changed() {
                change = Some(json!({"colorShift": shift}));
            }
        });
    });
    if let Some(p) = change {
        app.run("object.blend.options", p).ok();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let blend = crate::menus::enabled(app, "object.blend.release");
    buttons(app, ui, &[(tl!("Make"), "object.blend.make")]);
    widgets::divider(ui);
    ui.add_enabled_ui(blend || selection_len(app) >= 2, |ui| settings(app, ui));
    widgets::label_row(ui, tl!("Direction:"), LABEL_W, |ui| buttons(app, ui, &[(tl!("Reverse Front to Back"), "object.blend.reverseFrontToBack")]));
    widgets::divider(ui);
    buttons(app, ui, &[(tl!("Reverse Spine"), "object.blend.reverseSpine"), (tl!("Replace Spine"), "object.blend.replaceSpine")]);
    widgets::divider(ui);
    buttons(app, ui, &[(tl!("Expand"), "object.blend.expand"), (tl!("Release"), "object.blend.release")]);
}
