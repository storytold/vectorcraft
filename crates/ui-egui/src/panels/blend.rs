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

/// Is a blend made without a spine selected? Each selected object counts as its nearest blend,
/// itself or an ancestor, as the Object › Blend commands resolve it, so a key selected with Direct
/// Selection counts as its blend.
fn selects_blend_without_spine(app: &VectorcraftApp) -> bool {
    let Some(st) = app.session.active().filter(|st| st.selection.symbol_part.is_none()) else { return false };
    st.selection.objects.iter().any(|id| {
        let ancestry = st.doc.ancestry(*id).unwrap_or_default();
        let blend = ancestry.iter().rev().find_map(|a| match st.doc.node(*a).map(|n| &n.kind) {
            Some(NodeKind::Blend { spec, .. }) => Some(spec.no_spine),
            _ => None,
        });
        blend == Some(true)
    })
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

/// The Ramp or Shift slider: `key` of `object.blend.options` as a percentage from `min` to 100.
/// A drag, a press on the rail included, previews the selected blends and its release records one
/// undo step; a change without a drag, from the keyboard, records one at once.
fn percent_slider(app: &mut VectorcraftApp, ui: &mut Ui, key: &str, value: Option<f64>, min: f64) {
    let mut v = value.unwrap_or(0.0).clamp(min, 100.0);
    let resp = widgets::slider(ui, egui::Slider::new(&mut v, min..=100.0).suffix("%"));
    let phase = if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
        Live::Released
    } else if resp.changed() {
        Live::Dragging
    } else {
        return;
    };
    crate::panels::live_run(app, "Blend Options", "object.blend.options", json!({ key: v }), phase);
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
    // Position easing needs a spine to run along: a replaced one, or Align to Path. A blend made
    // without a spine has none, and easing is refused while one is selected.
    let spined =
        info["hasSpine"] != false && (info["spine"]["explicit"] == true || info["orientation"] == "path") && !selects_blend_without_spine(app);
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
        widgets::label_row(ui, tl!("Ramp:"), LABEL_W, |ui| percent_slider(app, ui, "ramp", info["ramp"].as_f64(), 0.0));
    });
    ui.add_enabled_ui(selected, |ui| {
        widgets::label_row(ui, tl!("Shift:"), LABEL_W, |ui| percent_slider(app, ui, "colorShift", info["colorShift"].as_f64(), -100.0));
    });
    if let Some(p) = change {
        app.run("object.blend.options", p).ok();
    }
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
