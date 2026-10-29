//! Object → Blend → Blend Options (also the Blend tool's double-click, Alt-click and toolbar
//! button): Spacing (Smooth Color, Specified Steps or Specified Distance) with its value,
//! Orientation (Align to Page or Align to Path), Spine (Auto, Always or Never), the Blend panel's
//! easing (Type, Ramp, Midpoint and the independent color Shift), and Direction and Reverse Spine,
//! opened on the selected blend's options and previewed live on it; OK keeps the preview as one
//! undo step (`object.blend.options`). With no blend selected it sets the options new blends start
//! with (the easing is per blend).
//!
//! Fields: `spacing` (`smooth` | `steps` | `distance`), `steps` (1..1000), `distance` (pt),
//! `orientation` (`page` | `path`), `spineMode` (`auto` | `always` | `never`), `easingType`,
//! `ramp`, `midpoint`, `colorShift`, `preview`, and `__target` (`blend` or `defaults`, from
//! `object.blend.info`).

use egui::{CornerRadius, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Blend Options.
pub const KIND: &str = "blendOptions";

const CMD: &str = "object.blend.options";

/// Width of the label column.
const LABEL_W: f32 = 82.0;

/// Spacing modes: (value, label).
pub(crate) const SPACING: [(&str, &str); 3] = [("smooth", "Smooth Color"), ("steps", "Specified Steps"), ("distance", "Specified Distance")];

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Blend Options").into(), body, confirm, preview: true, min_width: 300.0, ..DialogSpec::FORM };

/// Open Blend Options on the selected blend's options (else those new blends start with).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let mut fields = app.session.execute("object.blend.info", &json!({})).map_err(|e| e.to_string())?;
    let target = fields["target"].clone();
    let Some(o) = fields.as_object_mut() else { return Err("no blend options".into()) };
    o.retain(|k, _| {
        matches!(k.as_str(), "spacing" | "steps" | "distance" | "orientation" | "spineMode" | "easingType" | "ramp" | "midpoint" | "colorShift")
    });
    o.insert("__target".into(), target);
    o.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

/// Does the dialog edit a blend (rather than the options new blends start with)?
fn on_blend(d: &Dialog) -> bool {
    d.str("__target") == "blend"
}

/// `object.blend.options` parameters from the fields. The easing is per blend, so it is only sent
/// for one (`__target` `blend`); with no blend selected the engine refuses it.
fn params(d: &Dialog) -> Value {
    let mut p = json!({"spacing": d.str("spacing"), "orientation": d.str("orientation"), "spineMode": d.str("spineMode")});
    match d.str("spacing").as_str() {
        "steps" => p["steps"] = json!(d.f64("steps", 5.0).round().clamp(1.0, 1000.0)),
        "distance" => p["distance"] = json!(d.f64("distance", 10.0)),
        _ => {}
    }
    if on_blend(d) {
        p["easingType"] = json!(d.str("easingType"));
        p["ramp"] = json!(d.f64("ramp", 0.0));
        p["midpoint"] = json!(d.f64("midpoint", 50.0));
        p["colorShift"] = json!(d.f64("colorShift", 0.0));
    }
    p
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    let mode = d.str("spacing");
    let shown = SPACING.iter().find(|(v, _)| *v == mode).map_or(SPACING[0].1, |(_, l)| l);
    let labels = SPACING.map(|(_, l)| l);
    widgets::label_row(ui, tl!("Spacing:"), LABEL_W, |ui| {
        if let Some((value, _)) = widgets::dropdown(ui, "blend-spacing", shown, &labels, 150.0).and_then(|i| SPACING.get(i)) {
            d.fields.insert("spacing".into(), json!(value));
        }
        ui.add_space(6.0);
        match mode.as_str() {
            "distance" => {
                form::length(ui, d, "distance", unit, 72.0);
            }
            // Smooth Color picks the count itself: the field shows the steps, disabled.
            m => {
                let steps = d.f64("steps", 5.0);
                ui.add_enabled_ui(m == "steps", |ui| {
                    if let Some(n) = widgets::range_field(ui, "blend-steps", steps, 1.0..=1000.0, "", 0, 72.0) {
                        d.fields.insert("steps".into(), json!(n));
                    }
                });
            }
        }
    });
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Orientation:"), LABEL_W, |ui| {
        let path = d.str("orientation") == "path";
        for (value, along, tip) in [("page", false, tl!("Align to Page")), ("path", true, tl!("Align to Path"))] {
            if orientation_button(ui, along, path == along, tip) {
                d.fields.insert("orientation".into(), json!(value));
            }
        }
    });
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Spine:"), LABEL_W, |ui| {
        let modes = ["auto", "always", "never"];
        let labels = [tl!("Auto"), tl!("Always"), tl!("Never")];
        let mode = d.str("spineMode");
        let index = modes.iter().position(|m| *m == mode).unwrap_or(0);
        if let Some(i) = widgets::dropdown(ui, "blend-spine", labels[index], &labels, 150.0)
            && let Some(mode) = modes.get(i)
        {
            d.fields.insert("spineMode".into(), json!(mode));
        }
    });
    if on_blend(d) {
        // The easing rows match the Blend panel's: Type, Ramp and Midpoint move the steps between
        // any two keys, spine or not, and Shift moves the colours on their own.
        ui.add_space(6.0);
        widgets::label_row(ui, tl!("Type:"), LABEL_W, |ui| {
            let modes = ["easeIn", "easeOut", "easeInOut", "easeOutIn"];
            let labels = [tl!("Ease In"), tl!("Ease Out"), tl!("Ease In Out"), tl!("Ease Out In")];
            let mode = d.str("easingType");
            let index = modes.iter().position(|m| *m == mode).unwrap_or(0);
            if let Some(i) = widgets::dropdown(ui, "blend-easing", labels[index], &labels, 150.0)
                && let Some(mode) = modes.get(i)
            {
                d.fields.insert("easingType".into(), json!(mode));
            }
        });
        ui.add_space(6.0);
        widgets::label_row(ui, tl!("Ramp:"), LABEL_W, |ui| {
            let mut ramp = d.f64("ramp", 0.0);
            if ui.add(egui::Slider::new(&mut ramp, 0.0..=100.0).suffix("%")).changed() {
                d.fields.insert("ramp".into(), json!(ramp));
            }
        });
        ui.add_space(6.0);
        widgets::label_row(ui, tl!("Midpoint:"), LABEL_W, |ui| {
            let mut midpoint = d.f64("midpoint", 50.0);
            if ui.add(egui::Slider::new(&mut midpoint, 1.0..=99.0).suffix("%")).changed() {
                d.fields.insert("midpoint".into(), json!(midpoint));
            }
        });
        ui.add_space(6.0);
        widgets::label_row(ui, tl!("Shift:"), LABEL_W, |ui| {
            let mut shift = d.f64("colorShift", 0.0);
            if ui.add(egui::Slider::new(&mut shift, -100.0..=100.0).suffix("%")).changed() {
                d.fields.insert("colorShift".into(), json!(shift));
            }
        });
        // Direction and Reverse Spine act on the blend at once, not as dialog fields: roll the
        // preview back, run the command as its own undo step, and preview the fields again next
        // frame on top of the result.
        ui.add_space(6.0);
        widgets::label_row(ui, tl!("Direction:"), LABEL_W, |ui| {
            let cmd = "object.blend.reverseFrontToBack";
            let on = crate::menus::enabled(app, cmd);
            let button = ui.add_enabled_ui(on, |ui| widgets::flat_button_icon(ui, "dc-swap", "Reverse Front to Back", ui.available_width()));
            if button.inner.clicked() {
                reverse(app, d, cmd);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W);
            let cmd = "object.blend.reverseSpine";
            let on = crate::menus::enabled(app, cmd);
            let width = ui.available_width().max(120.0);
            if ui.add_enabled_ui(on, |ui| widgets::flat_button(ui, tl!("Reverse Spine"), width)).inner.clicked() {
                reverse(app, d, cmd);
            }
        });
        let p = params(d);
        form::preview(app, ui, d, "Blend Options", CMD, p);
    }
    false
}

/// Run a blend command from the dialog: cancel the live preview's undo step, run the command as
/// its own step, and let the next frame preview the dialog's fields on top of the result.
fn reverse(app: &mut VectorcraftApp, d: &mut Dialog, cmd: &str) {
    let _ = app.session.cancel_interaction();
    d.fields.remove(form::PREVIEWED);
    app.run(cmd, json!({})).ok();
}

/// An orientation toggle drawn in code: three bars on a curve, upright (Align to Page) or square
/// to the curve (Align to Path), in a pressed well when `selected`. Screen readers name it `tip`.
/// Returns clicked.
pub(crate) fn orientation_button(ui: &mut egui::Ui, along: bool, selected: bool, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let bg = if selected {
        t.tool_active
    } else if resp.hovered() {
        t.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), bg);
    let color = if selected { t.text } else { t.icon };
    // An arch through the button: y = c + a·x² with x in -1..1.
    let c = rect.center() + vec2(0.0, -2.0);
    let at = |x: f32| pos2(c.x + 9.0 * x, c.y + 6.0 * x * x);
    let curve: Vec<egui::Pos2> = (0..=12).map(|i| at(-1.0 + i as f32 / 6.0)).collect();
    ui.painter().add(egui::Shape::line(curve, Stroke::new(1.0, color)));
    for x in [-0.8f32, 0.0, 0.8] {
        let p = at(x);
        // Normal to the arch at x (pointing up), or straight up.
        let n = if along { vec2(-12.0 * x / 9.0, -1.0).normalized() } else { vec2(0.0, -1.0) };
        ui.painter().line_segment([p, p + n * 7.0], Stroke::new(2.0, color));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, tip));
    resp.on_hover_text(tip).clicked()
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if on_blend(d) { form::commit_preview(app, CMD, params(d)) } else { run_and_close(app, CMD, params(d)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::NodeKind;
    use vectorcraft_doc::live::{BlendOrientation, BlendSpacing};
    use vectorcraft_engine::Session;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    fn spec(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::BlendSpec {
        match &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
            NodeKind::Blend { spec, .. } => (**spec).clone(),
            _ => panic!("not a blend"),
        }
    }

    #[test]
    fn opens_on_the_blend_previews_and_keeps_smooth_color() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let id = app.run("object.blend.make", json!({"smooth": true})).unwrap()["id"].as_u64().unwrap();
        // The menu item opens the dialog on the blend's own options.
        crate::menus::invoke(&mut app, CMD, json!({}));
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        frame(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("spacing"), "smooth");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("spineMode"), "auto");
        // OK without changes keeps Smooth Color (it used to become 5 steps).
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(spec(&app, id).spacing, BlendSpacing::SmoothColor);
        // Specified Steps and Align to Path preview on the blend; Cancel rolls them back.
        app.run("ui.blendOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("spacing".into(), json!("steps"));
        d.fields.insert("steps".into(), json!(3));
        d.fields.insert("orientation".into(), json!("path"));
        frame(&mut app);
        assert_eq!(spec(&app, id).spacing, BlendSpacing::Steps(3), "previewed");
        crate::dialogs::cancel(&mut app);
        assert_eq!(spec(&app, id).spacing, BlendSpacing::SmoothColor);
        // OK keeps them as one undo step.
        app.run("ui.blendOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("spacing".into(), json!("distance"));
        d.fields.insert("distance".into(), json!(25));
        frame(&mut app);
        let depth = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(spec(&app, id).spacing, BlendSpacing::Distance(25.0));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
    }

    #[test]
    fn easing_rows_match_the_blend_panel() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let id = app.run("object.blend.make", json!({"steps": 4, "spineMode": "always"})).unwrap()["id"].as_u64().unwrap();
        app.run("ui.blendOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(d.str("easingType") == "easeIn", "the easing rows read the blend");
        // Type, Ramp, Midpoint and the color Shift preview like the panel's.
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("easingType".into(), json!("easeOut"));
        d.fields.insert("ramp".into(), json!(40.0));
        d.fields.insert("midpoint".into(), json!(25.0));
        d.fields.insert("colorShift".into(), json!(-20.0));
        frame(&mut app);
        let s = spec(&app, id);
        assert_eq!(
            (s.ease.kind.id(), (s.ease.ramp * 100.0) as i32, (s.ease.midpoint * 100.0) as i32, s.color_shift as i32),
            ("easeOut", 40, 25, -20),
            "previewed"
        );
        crate::dialogs::confirm(&mut app).unwrap();
        let s = spec(&app, id);
        assert_eq!(
            (s.ease.kind.id(), (s.ease.ramp * 100.0) as i32, (s.ease.midpoint * 100.0) as i32, s.color_shift as i32),
            ("easeOut", 40, 25, -20)
        );
    }

    #[test]
    fn direction_and_reverse_spine_keep_the_preview_and_undo_separately() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let id = app.run("object.blend.make", json!({"steps": 4, "spineMode": "always"})).unwrap()["id"].as_u64().unwrap();
        let keys = |app: &VectorcraftApp| -> Vec<vectorcraft_doc::NodeId> {
            app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().children().unwrap().iter().map(|k| k.id).collect()
        };
        let before = keys(&app);
        app.run("ui.blendOptions", json!({})).unwrap();
        // A field change has a live preview running.
        app.ui.dialog.as_mut().unwrap().fields.insert("ramp".into(), json!(40.0));
        frame(&mut app);
        assert!((spec(&app, id).ease.ramp - 0.4).abs() < 1e-9, "previewed");
        let depth = app.session.doc().unwrap().history.undo.len();
        // The Direction button runs the command on its own: the preview rolls back first, the
        // command lands as one undo step, and the next frame previews the fields again.
        let mut d = app.ui.dialog.take().unwrap();
        reverse(&mut app, &mut d, "object.blend.reverseFrontToBack");
        app.ui.dialog = Some(d);
        assert_eq!(keys(&app), before.iter().rev().copied().collect::<Vec<_>>());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
        frame(&mut app);
        assert!((spec(&app, id).ease.ramp - 0.4).abs() < 1e-9, "the fields preview again on top");
        // Cancel rolls the preview back but keeps the reversal.
        crate::dialogs::cancel(&mut app);
        assert_eq!(keys(&app), before.iter().rev().copied().collect::<Vec<_>>());
        assert!(spec(&app, id).ease.ramp.abs() < 1e-9, "the preview rolled back");
    }

    #[test]
    fn with_nothing_selected_it_sets_what_new_blends_start_with() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("ui.blendOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("__target"), "defaults");
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("spacing".into(), json!("steps"));
        d.fields.insert("steps".into(), json!(7));
        d.fields.insert("orientation".into(), json!("path"));
        d.fields.insert("spineMode".into(), json!("never"));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 0, "width": 20, "height": 20})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let id = app.run("object.blend.make", json!({})).unwrap()["id"].as_u64().unwrap();
        let s = spec(&app, id);
        assert_eq!((s.spacing, s.orientation), (BlendSpacing::Steps(7), BlendOrientation::AlignToPath));
        assert_eq!(s.spine_mode, vectorcraft_doc::live::BlendSpineMode::Never);
    }
}
