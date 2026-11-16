//! Blend panel (Window → Blend): Make, Create Spine (whether new blends get a spine), the Spacing
//! and Orientation of the selected blend (with no blend selected, the options new blends start
//! with), Reverse Front to Back, Reverse Spine, Replace Spine, Expand and Release. Each control
//! runs its Object → Blend command (`object.blend.make`, `object.blend.createSpine`,
//! `object.blend.options` and the others), so a change of the selected blend's spacing or
//! orientation shows on the canvas at once as one undo step. Make is dimmed until two or more
//! objects are selected. Create Spine is always available. Spacing and Orientation are dimmed until
//! a blend or two or more objects are selected, and the other buttons are dimmed until a blend is
//! selected. Easing and Reverse Spine are dimmed for a blend made without a spine.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_doc::live::BlendSpineMode;

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

/// Can `cmd` run on the selection? Make needs two or more objects, the other commands need a
/// selected blend, and Reverse Spine needs every selected blend to have a spine.
fn available(app: &VectorcraftApp, cmd: &str) -> bool {
    crate::menus::enabled(app, cmd)
        && (cmd != "object.blend.make" || selection_len(app) >= 2)
        && (cmd != "object.blend.reverseSpine" || !selects_blend_without_spine(app))
}

/// Is a blend made without a spine selected?
fn selects_blend_without_spine(app: &VectorcraftApp) -> bool {
    let Ok(st) = app.session.doc() else { return false };
    st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Blend { spec, .. }) if spec.no_spine))
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

#[derive(Clone, Default)]
struct PendingEase {
    doc_uid: Option<u64>,
    id: Option<u64>,
    easing_type: Option<String>,
    ramp: Option<f64>,
    color_shift: Option<f64>,
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
    let mut spacing_change = None;
    let shown = SPACING.iter().find(|(v, _)| *v == mode).map_or(SPACING[0].1, |(_, l)| l);
    let labels = SPACING.map(|(_, l)| l);
    widgets::label_row(ui, tl!("Spacing:"), LABEL_W, |ui| {
        let width = ui.available_width().max(FIELD_W);
        if let Some((value, _)) = widgets::dropdown(ui, "blend-panel-spacing", shown, &labels, width).and_then(|i| SPACING.get(i)) {
            spacing_change = Some(spacing_params(value, steps, distance));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W);
        if mode == "distance" {
            if let Some(d) = widgets::num_field(ui, "blend-panel-distance", Some(distance), unit, FIELD_W) {
                spacing_change = Some(spacing_params("distance", steps, d));
            }
        } else {
            // Smooth Color picks the count itself: the field shows the steps, disabled.
            ui.add_enabled_ui(mode == "steps", |ui| {
                if let Some(n) = widgets::range_field(ui, "blend-panel-steps", steps, 1.0..=1000.0, "", 0, FIELD_W) {
                    spacing_change = Some(spacing_params("steps", n, distance));
                }
            });
        }
    });
    widgets::label_row(ui, tl!("Orientation:"), LABEL_W, |ui| {
        for (value, along, tip) in [("page", false, tl!("Align to Page")), ("path", true, tl!("Align to Path"))] {
            if orientation_button(ui, along, path == along, tip) {
                spacing_change = Some(json!({ "orientation": value }));
            }
        }
    });
    if let Some(p) = spacing_change {
        app.run("object.blend.options", p).ok();
    }

    let selected = info["target"] == "blend";
    let doc_uid = app.session.active().map(|d| d.uid);
    let blend_id = info["id"].as_u64();
    let mut pending = crate::panels::pstate::<PendingEase>(ui.ctx(), "blend-pending-ease");
    if pending.doc_uid != doc_uid || pending.id != blend_id {
        pending = PendingEase { doc_uid, id: blend_id, ..Default::default() };
    }

    let preview = app.ui.blend_preview;
    if preview && selected {
        let mut p = json!({});
        if let Some(et) = pending.easing_type.take() {
            p["easingType"] = json!(et);
        }
        if let Some(r) = pending.ramp.take() {
            p["ramp"] = json!(r);
        }
        if let Some(cs) = pending.color_shift.take() {
            p["colorShift"] = json!(cs);
        }
        if p.as_object().is_some_and(|o| !o.is_empty()) {
            app.run("object.blend.options", p).ok();
        }
    }

    // Position easing needs a spine to run along: a replaced one, or Align to Path. A blend made
    // without a spine has none.
    let spined = info["hasSpine"] != false && (info["spine"]["explicit"] == true || info["orientation"] == "path");
    ui.add_enabled_ui(selected && spined, |ui| {
        let modes = ["linear", "easeIn", "easeOut", "easeInOut", "easeOutIn"];
        let labels = [tl!("Linear"), tl!("Ease In"), tl!("Ease Out"), tl!("Ease In Out"), tl!("Ease Out In")];
        let current_mode = pending.easing_type.as_deref().or_else(|| info["easingType"].as_str()).unwrap_or(modes[1]);
        let index = modes.iter().position(|m| *m == current_mode).unwrap_or(1);
        widgets::label_row(ui, tl!("Type:"), LABEL_W, |ui| {
            if let Some(i) = widgets::dropdown(ui, "blend-panel-easing", labels[index], &labels, ui.available_width().max(FIELD_W))
                && let Some(mode) = modes.get(i)
            {
                if preview {
                    app.run("object.blend.options", json!({"easingType": mode})).ok();
                } else {
                    pending.easing_type = Some(mode.to_string());
                }
            }
        });
        widgets::label_row(ui, tl!("Ramp:"), LABEL_W, |ui| {
            let mut ramp = pending.ramp.or_else(|| info["ramp"].as_f64().filter(|n| n.is_finite())).map_or(0.0, |n| n.clamp(0.0, 100.0));
            if widgets::slider(ui, egui::Slider::new(&mut ramp, 0.0..=100.0).suffix("%").step_by(1.0)).changed() {
                let clamped = ramp.clamp(0.0, 100.0);
                if preview {
                    app.run("object.blend.options", json!({"ramp": clamped})).ok();
                } else {
                    pending.ramp = Some(clamped);
                }
            }
        });
    });
    ui.add_enabled_ui(selected, |ui| {
        widgets::label_row(ui, tl!("Shift:"), LABEL_W, |ui| {
            let mut shift =
                pending.color_shift.or_else(|| info["colorShift"].as_f64().filter(|n| n.is_finite())).map_or(0.0, |n| n.clamp(-100.0, 100.0));
            if widgets::slider(ui, egui::Slider::new(&mut shift, -100.0..=100.0).suffix("%").step_by(1.0)).changed() {
                let clamped = shift.clamp(-100.0, 100.0);
                if preview {
                    app.run("object.blend.options", json!({"colorShift": clamped})).ok();
                } else {
                    pending.color_shift = Some(clamped);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W);
            let cur_preview = app.ui.blend_preview;
            if widgets::check(ui, tl!("Preview"), cur_preview, true) {
                let next = !cur_preview;
                app.ui.blend_preview = next;
                if next {
                    let mut p = json!({});
                    if let Some(et) = pending.easing_type.take() {
                        p["easingType"] = json!(et);
                    }
                    if let Some(r) = pending.ramp.take() {
                        p["ramp"] = json!(r);
                    }
                    if let Some(cs) = pending.color_shift.take() {
                        p["colorShift"] = json!(cs);
                    }
                    if p.as_object().is_some_and(|o| !o.is_empty()) {
                        app.run("object.blend.options", p).ok();
                    }
                }
            }
        });
    });
    crate::panels::set_pstate(ui.ctx(), "blend-pending-ease", pending);
}

/// Create Spine: whether new blends get a spine. It shows the setting new blends take, whatever is
/// selected, and a change runs `object.blend.createSpine`; blends already made keep theirs.
fn create_spine(app: &mut VectorcraftApp, ui: &mut Ui) {
    let current = app.session.prefs.blend_options.unwrap_or_default().create_spine;
    let modes = BlendSpineMode::ALL;
    let labels = [tl!("Auto"), tl!("Always"), tl!("Never")];
    let shown = modes.iter().position(|m| *m == current).and_then(|i| labels.get(i)).copied().unwrap_or(labels[0]);
    let mut change = None;
    widgets::label_row(ui, tl!("Create Spine:"), LABEL_W, |ui| {
        if let Some(mode) =
            widgets::dropdown(ui, "blend-panel-create-spine", shown, &labels, ui.available_width().max(FIELD_W)).and_then(|i| modes.get(i))
        {
            change = Some(*mode);
        }
    });
    if let Some(mode) = change {
        app.run("object.blend.createSpine", json!({ "mode": mode.id() })).ok();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let blend = crate::menus::enabled(app, "object.blend.release");
    buttons(app, ui, &[(tl!("Make"), "object.blend.make")]);
    create_spine(app, ui);
    widgets::divider(ui);
    ui.add_enabled_ui(blend || selection_len(app) >= 2, |ui| settings(app, ui));
    widgets::label_row(ui, tl!("Direction:"), LABEL_W, |ui| buttons(app, ui, &[(tl!("Reverse Front to Back"), "object.blend.reverseFrontToBack")]));
    widgets::divider(ui);
    buttons(app, ui, &[(tl!("Reverse Spine"), "object.blend.reverseSpine"), (tl!("Replace Spine"), "object.blend.replaceSpine")]);
    widgets::divider(ui);
    buttons(app, ui, &[(tl!("Expand"), "object.blend.expand"), (tl!("Release"), "object.blend.release")]);
}
