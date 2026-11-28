//! Blend panel (Window → Blend): Make, Create Spine (whether new blends get a spine), the Spacing
//! and Orientation of the selected blend (with no blend selected, the options new blends start
//! with), Reverse Front to Back, Reverse Spine, Replace Spine, Expand and Release. Each control
//! runs its Object → Blend command (`object.blend.make`, `object.blend.createSpine`,
//! `object.blend.options` and the others), so a change of the selected blend's spacing or
//! orientation shows on the canvas at once as one undo step. Make is dimmed until two or more
//! objects are selected. Create Spine is always available. Spacing and Orientation are dimmed until
//! a blend or two or more objects are selected, and the other buttons are dimmed until a blend is
//! selected. Easing and Reverse Spine are dimmed for a blend made without a spine.

use std::ops::RangeInclusive;

use craft::behavior::IconId;
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, FieldLabelRole, Props as _, Show as _, ShowContainer as _, SideLabel, clicked};
use craft::ui::display::Divider;
use craft::ui::form::{Field, FieldGroup};
use craft::ui::form::{SliderField, SliderFieldLayout};
use craft::ui::layout_components::Row;
use craft::ui::pickers::Select;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_doc::live::BlendSpineMode;

use super::selection_len;
use crate::VectorcraftApp;
use crate::dialogs::blend_options::SPACING;
// PLAN: assumed `edits` module path for wg.A16 and `widgets::general_lengths` for cv.B5b
use crate::canvas::general_lengths;
use crate::edits::{EditPath, apply_edits};

pub const ID: &str = "blend";

/// A button running `cmd`, dimmed while `cmd` is unavailable on the selection.
fn action(app: &mut VectorcraftApp, ui: &mut Ui, label: &'static str, cmd: &str) {
    if clicked(&Button::new(label).enabled(available(app, cmd)).show(ui)) {
        app.run(cmd, json!({})).ok();
    }
}

/// Buttons side by side, each (label, command).
fn actions(app: &mut VectorcraftApp, ui: &mut Ui, row: &[(&'static str, &str)]) {
    Row::new().fill_width().show(ui, |ui, _| {
        for &(label, cmd) in row {
            action(app, ui, label, cmd);
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
    let mut find = st.doc.nearest_finder();
    st.selection.objects.iter().any(|id| {
        let blend = find.nearest(*id, |n| matches!(n.kind, NodeKind::Blend { .. }));
        matches!(blend.map(|n| &n.kind), Some(NodeKind::Blend { spec, .. }) if spec.no_spine)
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

/// The Ramp or Shift slider: `key` of `object.blend.options` as a percentage over `range`. A drag
/// previews the selected blends and its release records one undo step; a change without a drag,
/// from the keyboard, records one at once.
fn percent_slider(
    app: &mut VectorcraftApp,
    ui: &mut Ui,
    label: &'static str,
    key: &'static str,
    value: Option<f64>,
    range: RangeInclusive<f64>,
    on: bool,
) {
    Field::new(label).enabled(on).show(ui, |ui, _| {
        let shown = value.unwrap_or(0.0).clamp(*range.start(), *range.end());
        let out =
            SliderField::new(label, Bind::given(shown), range.clone()).layout(SliderFieldLayout::Inline).unit("%").step(1.0).enabled(on).show(ui);
        apply_edits(app, &out.edit, EditPath::Preview, |app, v, phase| {
            crate::panels::live_run(app, "Blend Options", "object.blend.options", json!({ key: *v }), phase);
        });
    });
}

/// Spacing and Orientation, as `object.blend.info` reports them; a change runs
/// `object.blend.options`.
fn settings(app: &mut VectorcraftApp, ui: &mut Ui) {
    let info = app.session.execute("object.blend.info", &json!({})).unwrap_or_default();
    let mode = info["spacing"].as_str().unwrap_or("smooth");
    let steps = info["steps"].as_f64().unwrap_or(5.0);
    let distance = info["distance"].as_f64().unwrap_or(10.0);
    let mut change = None;
    let spacings: Vec<Entry<&'static str>> = SPACING.iter().map(|(v, l)| Entry::new(*v, *l)).collect();
    Field::new(tl!("Spacing:")).show(ui, |ui, _| {
        let picked = Select::new(tl!("Spacing:"), Bind::given(mode_id(mode)), &spacings).show(ui).events.committed().copied();
        if let Some(value) = picked.filter(|value| *value != mode) {
            change = Some(spacing_params(value, steps, distance));
        }
    });
    if mode == "distance" {
        Field::new(tl!("Distance:")).show(ui, |ui, _| {
            let out = NumberField::length(tl!("Distance:"), Bind::given(Some(distance)), general_lengths(app)).show(ui);
            if let Some(d) = out.events.committed().copied().flatten() {
                change = Some(spacing_params("distance", steps, d));
            }
        });
    } else {
        // Smooth Color picks the count itself: the field shows the steps, disabled.
        let on = mode == "steps";
        Field::new(tl!("Steps:")).enabled(on).show(ui, |ui, _| {
            let out = NumberField::new(tl!("Steps:"), Bind::given(Some(steps))).range(1.0..=1000.0).commit_decimals(0).enabled(on).show(ui);
            if let Some(n) = out.events.committed().copied().flatten() {
                change = Some(spacing_params("steps", n, distance));
            }
        });
    }
    let orientations = [
        Entry::new("page", tl!("Align to Page")).icon(IconId::new("dc-blend-align-page")),
        Entry::new("path", tl!("Align to Path")).icon(IconId::new("dc-blend-align-path")),
    ];
    let orientation = if info["orientation"] == "path" { "path" } else { "page" };
    Field::new(tl!("Orientation:")).show(ui, |ui, _| {
        let picked = Segmented::new(tl!("Orientation:"), Bind::given(orientation), &orientations).show(ui).events.committed().copied();
        if let Some(value) = picked.filter(|value| *value != orientation) {
            change = Some(json!({ "orientation": value }));
        }
    });
    let selected = info["target"] == "blend";
    // Position easing needs a spine to run along: a replaced one, or Align to Path. A blend made
    // without a spine has none, and easing is refused while one is selected.
    let spined =
        info["hasSpine"] != false && (info["spine"]["explicit"] == true || info["orientation"] == "path") && !selects_blend_without_spine(app);
    let modes = ["easeIn", "easeOut", "easeInOut", "easeOutIn"];
    let labels = [tl!("Ease In"), tl!("Ease Out"), tl!("Ease In Out"), tl!("Ease Out In")];
    let easings: Vec<Entry<&'static str>> = modes.iter().zip(labels).map(|(m, l)| Entry::new(*m, l)).collect();
    let easing = modes.iter().copied().find(|m| Some(*m) == info["easingType"].as_str()).unwrap_or(modes[0]);
    Field::new(tl!("Type:")).enabled(selected && spined).show(ui, |ui, _| {
        let picked = Select::new(tl!("Type:"), Bind::given(easing), &easings).enabled(selected && spined).show(ui).events.committed().copied();
        if let Some(value) = picked.filter(|value| *value != easing) {
            change = Some(json!({"easingType": value}));
        }
    });
    percent_slider(app, ui, tl!("Ramp:"), "ramp", info["ramp"].as_f64(), 0.0..=100.0, selected && spined);
    percent_slider(app, ui, tl!("Shift:"), "colorShift", info["colorShift"].as_f64(), -100.0..=100.0, selected);
    if let Some(p) = change {
        app.run("object.blend.options", p).ok();
    }
}

/// The `SPACING` id a Spacing select shows for `mode`, Smooth Color for an unknown one.
fn mode_id(mode: &str) -> &'static str {
    SPACING.iter().find(|(v, _)| *v == mode).map_or(SPACING[0].0, |(v, _)| v)
}

/// Create Spine: whether new blends get a spine. It shows the setting new blends take, whatever is
/// selected, and a change runs `object.blend.createSpine`; blends already made keep theirs.
fn create_spine(app: &mut VectorcraftApp, ui: &mut Ui) {
    let current = app.session.prefs.blend_options.unwrap_or_default().create_spine.id();
    let labels = [tl!("Auto"), tl!("Always"), tl!("Never")];
    let entries: Vec<Entry<&'static str>> = BlendSpineMode::ALL.iter().zip(labels).map(|(m, l)| Entry::new(m.id(), l)).collect();
    let mut picked = None;
    Field::new(tl!("Create Spine:")).show(ui, |ui, _| {
        picked = Select::new(tl!("Create Spine:"), Bind::given(current), &entries).show(ui).events.committed().copied();
    });
    if let Some(mode) = picked.filter(|mode| *mode != current) {
        app.run("object.blend.createSpine", json!({ "mode": mode })).ok();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let blend = crate::menus::enabled(app, "object.blend.release");
    action(app, ui, tl!("Make"), "object.blend.make");
    create_spine(app, ui);
    Divider::horizontal().show(ui);
    FieldGroup::new(tl!("Blend Options")).enabled(blend || selection_len(app) >= 2).show(ui, |ui, _| settings(app, ui));
    // The button keeps its own name: the label says what it changes.
    Field::new(tl!("Direction:"))
        .side(SideLabel::new().role(FieldLabelRole::Describes))
        .show(ui, |ui, _| action(app, ui, tl!("Reverse Front to Back"), "object.blend.reverseFrontToBack"));
    Divider::horizontal().show(ui);
    actions(app, ui, &[(tl!("Reverse Spine"), "object.blend.reverseSpine"), (tl!("Replace Spine"), "object.blend.replaceSpine")]);
    Divider::horizontal().show(ui);
    actions(app, ui, &[(tl!("Expand"), "object.blend.expand"), (tl!("Release"), "object.blend.release")]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.run("object.blend.createSpine", json!({"mode": "always"})).unwrap();
        for x in [10, 200] {
            app.run("shape.rectangle", json!({"x": x, "y": 30, "width": 30, "height": 30})).unwrap();
        }
        app.run("select.all", json!({})).unwrap();
        app.run("object.blend.make", json!({})).unwrap();
        app.run("object.blend.options", json!({"spacing": "smooth", "orientation": "path", "easingType": "easeIn", "ramp": 50})).unwrap();
        app
    }

    #[test]
    fn canceled_dropdown_choices_leave_blend_options_and_defaults_unchanged() {
        for label in ["Spacing:", "Type:", "Create Spine:"] {
            let app = app();
            let original = app.session.active().unwrap().doc.clone();
            let revision = app.session.active().unwrap().revision;
            let undo = app.session.active().unwrap().history.undo.len();
            let prefs = serde_json::to_value(app.session.prefs.blend_options).unwrap();
            let mut panel = Panel::new(app, show);
            panel.room(egui::vec2(400.0, 900.0));
            panel.click_role(label, Role::COMBO_BOX);
            panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
            panel.key(egui::Key::Escape, egui::Modifiers::NONE);
            {
                let app = panel.app();
                let state = app.session.active().unwrap();
                assert_eq!(state.doc, original, "canceling {label} keeps the blend");
                assert_eq!(state.revision, revision);
                assert_eq!(state.history.undo.len(), undo);
                assert_eq!(serde_json::to_value(app.session.prefs.blend_options).unwrap(), prefs);
            }
            panel.click_role(label, Role::COMBO_BOX);
            panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
            panel.key(egui::Key::Enter, egui::Modifiers::NONE);
            let info = panel.app().run("object.blend.info", json!({})).unwrap();
            if label == "Create Spine:" {
                assert_eq!(panel.app().session.prefs.blend_options.unwrap().create_spine, BlendSpineMode::Never);
                assert_eq!(panel.app().session.active().unwrap().doc, original);
                assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
            } else {
                assert_eq!(
                    if label == "Spacing:" { &info["spacing"] } else { &info["easingType"] },
                    if label == "Spacing:" { "steps" } else { "easeOut" }
                );
                assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
                panel.app().run("edit.undo", json!({})).unwrap();
                assert_eq!(panel.app().session.active().unwrap().doc, original);
            }
        }
    }

    #[test]
    fn canceled_step_and_distance_typing_leaves_the_blend_and_history_unchanged() {
        for (mode, label, key, typed, value) in [("steps", "Steps:", "steps", "12", 12.0), ("distance", "Distance:", "distance", "24 pt", 24.0)] {
            let mut app = app();
            app.run("object.blend.options", json!({"spacing": mode, "steps": 5, "distance": 10})).unwrap();
            let original = app.session.active().unwrap().doc.clone();
            let revision = app.session.active().unwrap().revision;
            let undo = app.session.active().unwrap().history.undo.len();
            let mut panel = Panel::new(app, show);
            panel.room(egui::vec2(400.0, 900.0));
            panel.click_role(label, Role::SPIN_BUTTON);
            panel.key(egui::Key::A, egui::Modifiers::COMMAND);
            panel.type_text(typed);
            panel.key(egui::Key::Escape, egui::Modifiers::NONE);
            {
                let app = panel.app();
                let state = app.session.active().unwrap();
                assert_eq!(state.doc, original, "canceling {label} keeps the blend");
                assert_eq!(state.revision, revision);
                assert_eq!(state.history.undo.len(), undo);
            }
            panel.click_role(label, Role::SPIN_BUTTON);
            panel.key(egui::Key::A, egui::Modifiers::COMMAND);
            panel.type_text(typed);
            panel.key(egui::Key::Enter, egui::Modifiers::NONE);
            assert_eq!(panel.app().run("object.blend.info", json!({})).unwrap()[key], value);
            assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo + 1);
            panel.app().run("edit.undo", json!({})).unwrap();
            assert_eq!(panel.app().session.active().unwrap().doc, original);
        }
    }
}
