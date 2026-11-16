//! Blend panel (Window → Blend): Make, Create Spine (whether new blends get a spine), the Spacing
//! and Orientation of the selected blend (with no blend selected, the options new blends start
//! with), Reverse Front to Back, Reverse Spine, Replace Spine, Expand and Release. Each control
//! runs its Object → Blend command (`object.blend.make`, `object.blend.createSpine`,
//! `object.blend.options` and the others), so a change of the selected blend's spacing or
//! orientation shows on the canvas at once as one undo step. Make is dimmed until two or more
//! objects are selected. Create Spine is always available. Spacing and Orientation are dimmed until
//! a blend or two or more objects are selected, and the other buttons are dimmed until a blend is
//! selected. Easing and Reverse Spine are dimmed for a blend made without a spine.

use std::collections::HashMap;

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_doc::live::BlendSpineMode;

use super::selection_len;
use crate::VectorcraftApp;
use crate::dialogs::blend_options::{SPACING, orientation_button};
use crate::widgets::{self, Live};

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
struct BlendEaseDraft {
    easing_type: Option<String>,
    ramp: Option<f64>,
    color_shift: Option<f64>,
    staged_active_easing_type: Option<String>,
    staged_active_ramp: Option<f64>,
    staged_active_color_shift: Option<f64>,
    rejected: bool,
}

impl BlendEaseDraft {
    fn is_empty(&self) -> bool {
        self.easing_type.is_none() && self.ramp.is_none() && self.color_shift.is_none()
    }

    fn clear(&mut self) {
        self.easing_type = None;
        self.ramp = None;
        self.color_shift = None;
        self.staged_active_easing_type = None;
        self.staged_active_ramp = None;
        self.staged_active_color_shift = None;
        self.rejected = false;
    }
}

#[derive(Clone, Default)]
struct PendingEaseMap {
    drafts: HashMap<String, BlendEaseDraft>,
}

fn flush_pending(app: &mut VectorcraftApp, blend_id: Option<u64>, draft: &mut BlendEaseDraft) {
    if draft.rejected || draft.is_empty() {
        return;
    }
    let mut p = json!({});
    if let Some(id) = blend_id {
        p["id"] = json!(id);
    }
    if let Some(ref et) = draft.easing_type {
        p["easingType"] = json!(et);
    }
    if let Some(r) = draft.ramp {
        p["ramp"] = json!(r);
    }
    if let Some(cs) = draft.color_shift {
        p["colorShift"] = json!(cs);
    }
    if p.as_object().is_some_and(|o| !o.is_empty()) {
        match app.run("object.blend.options", p) {
            Ok(_) => {
                draft.clear();
            }
            Err(e) => {
                draft.rejected = true;
                app.status(e);
            }
        }
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
    let key = format!("{}:{}", doc_uid.map(|u| u.to_string()).unwrap_or_default(), blend_id.map(|b| b.to_string()).unwrap_or_default());

    let mut pending_map = crate::panels::pstate::<PendingEaseMap>(ui.ctx(), "blend-pending-ease-map");
    let mut draft = pending_map.drafts.remove(&key).unwrap_or_default();

    let active_easing = info["easingType"].as_str().map(|s| s.to_string());
    let active_ramp = info["ramp"].as_f64().filter(|n| n.is_finite());
    let active_shift = info["colorShift"].as_f64().filter(|n| n.is_finite());

    // Synchronize staged values after external on-canvas edits:
    // If an active parameter changed externally since staging, invalidate its staged value
    // so Preview does not overwrite newer on-canvas edits, while retaining unchanged staged parameters.
    if let Some(ref staged_active) = draft.staged_active_easing_type {
        if active_easing.as_ref() != Some(staged_active) {
            draft.easing_type = None;
            draft.staged_active_easing_type = None;
        }
    }
    if let Some(staged_active) = draft.staged_active_ramp {
        if active_ramp != Some(staged_active) {
            draft.ramp = None;
            draft.staged_active_ramp = None;
        }
    }
    if let Some(staged_active) = draft.staged_active_color_shift {
        if active_shift != Some(staged_active) {
            draft.color_shift = None;
            draft.staged_active_color_shift = None;
        }
    }

    let preview = app.ui.blend_preview;
    if preview && selected {
        flush_pending(app, blend_id, &mut draft);
    }

    // Position easing needs a spine to run along: a replaced one, or Align to Path. A blend made
    // without a spine has none.
    let spined = info["hasSpine"] != false && (info["spine"]["explicit"] == true || info["orientation"] == "path");
    ui.add_enabled_ui(selected && spined, |ui| {
        let modes = ["linear", "easeIn", "easeOut", "easeInOut", "easeOutIn"];
        let labels = [tl!("Linear"), tl!("Ease In"), tl!("Ease Out"), tl!("Ease In Out"), tl!("Ease Out In")];
        let current_mode = draft.easing_type.as_deref().or(active_easing.as_deref()).unwrap_or(modes[1]);
        let index = modes.iter().position(|m| *m == current_mode).unwrap_or(1);
        widgets::label_row(ui, tl!("Type:"), LABEL_W, |ui| {
            if let Some(i) = widgets::dropdown(ui, "blend-panel-easing", labels[index], &labels, ui.available_width().max(FIELD_W))
                && let Some(mode) = modes.get(i)
            {
                if preview {
                    let mut param = json!({"easingType": mode});
                    if let Some(id) = blend_id {
                        param["id"] = json!(id);
                    }
                    app.run("object.blend.options", param).ok();
                } else {
                    draft.easing_type = Some(mode.to_string());
                    draft.staged_active_easing_type = active_easing.clone();
                    draft.rejected = false;
                }
            }
        });
        widgets::label_row(ui, tl!("Ramp:"), LABEL_W, |ui| {
            let mut ramp = draft.ramp.or(active_ramp).unwrap_or(0.0).clamp(0.0, 100.0);
            let resp = widgets::slider(ui, egui::Slider::new(&mut ramp, 0.0..=100.0).suffix("%").step_by(1.0));
            let phase = if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                Live::Released
            } else if resp.changed() {
                Live::Dragging
            } else {
                Live::Idle
            };
            let clamped = ramp.clamp(0.0, 100.0);
            if preview {
                if phase != Live::Idle {
                    let mut param = json!({"ramp": clamped});
                    if let Some(id) = blend_id {
                        param["id"] = json!(id);
                    }
                    crate::panels::live_run(app, "Blend Options", "object.blend.options", param, phase);
                }
            } else if resp.changed() {
                draft.ramp = Some(clamped);
                draft.staged_active_ramp = active_ramp;
                draft.rejected = false;
            }
        });
    });
    ui.add_enabled_ui(selected, |ui| {
        widgets::label_row(ui, tl!("Shift:"), LABEL_W, |ui| {
            let mut shift = draft.color_shift.or(active_shift).unwrap_or(0.0).clamp(-100.0, 100.0);
            let resp = widgets::slider(ui, egui::Slider::new(&mut shift, -100.0..=100.0).suffix("%").step_by(1.0));
            let phase = if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                Live::Released
            } else if resp.changed() {
                Live::Dragging
            } else {
                Live::Idle
            };
            let clamped = shift.clamp(-100.0, 100.0);
            if preview {
                if phase != Live::Idle {
                    let mut param = json!({"colorShift": clamped});
                    if let Some(id) = blend_id {
                        param["id"] = json!(id);
                    }
                    crate::panels::live_run(app, "Blend Options", "object.blend.options", param, phase);
                }
            } else if resp.changed() {
                draft.color_shift = Some(clamped);
                draft.staged_active_color_shift = active_shift;
                draft.rejected = false;
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W);
            let cur_preview = app.ui.blend_preview;
            if widgets::check(ui, tl!("Preview"), cur_preview, true) {
                let next = !cur_preview;
                app.ui.blend_preview = next;
                if next && selected {
                    draft.rejected = false;
                    flush_pending(app, blend_id, &mut draft);
                }
            }
        });
    });
    if !draft.is_empty() || draft.rejected {
        pending_map.drafts.insert(key, draft);
    }
    crate::panels::set_pstate(ui.ctx(), "blend-pending-ease-map", pending_map);
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
