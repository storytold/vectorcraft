//! Color Guide Options: the variation steps on each side of the harmony colours (1–20) and how far
//! they reach (Less … More), with the base colour's variations as a preview. OK sets the Color
//! Guide panel's options (`UiState::color_guide`).
//!
//! Fields: `steps` (1–20) and `amount` (0–100).

use egui::{Rect, Sense, vec2};
use serde_json::{Value, json};
use vectorcraft_color::harmony::{GuideOptions, variation};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::panels::{c32, color_guide::base_color};
use crate::state::Dialog;
use crate::theme::Tokens;

/// The dialog kind of Color Guide Options.
pub const KIND: &str = "colorGuideOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Color Guide Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// Open the dialog with the panel's current options.
pub fn open(app: &mut VectorcraftApp) {
    let o = app.ui.color_guide;
    app.ui.dialog = Some(Dialog::new(KIND, json!({"steps": o.steps, "amount": o.amount})));
}

/// The panel's options with the dialog's steps and amount.
fn options(app: &VectorcraftApp, d: &Dialog) -> GuideOptions {
    let o = app.ui.color_guide;
    GuideOptions { steps: d.f64("steps", o.steps as f64).round().max(1.0) as u32, amount: d.f64("amount", o.amount as f64) as f32, ..o }.clamped()
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let max = GuideOptions::MAX_STEPS as f64;
    let ramp = |x: f32| {
        let v = (60.0 + 160.0 * x) as u8;
        egui::Color32::from_rgb(v, v, v)
    };
    form::slider(ui, d, "steps", tl!("Steps:"), 1.0..=max, "", &ramp);
    form::slider(ui, d, "amount", tl!("Variation:"), 0.0..=100.0, "%", &ramp);
    form::slider_ends(ui, form::SLIDER_LABEL, (tl!("Less"), tl!("More")));
    ui.add_space(10.0);
    // Preview: the base colour's row of variations.
    let o = options(app, d);
    let base = base_color(app);
    let n = o.steps as i32;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let w = row.width() / (2 * n + 1) as f32;
    for (i, step) in (-n..=n).enumerate() {
        let r = Rect::from_min_size(row.min + vec2(i as f32 * w, 0.0), vec2(w, row.height()));
        ui.painter().rect_filled(r, 0.0, c32(&variation(base, &o, step)));
    }
    ui.painter().rect_stroke(row, 0.0, egui::Stroke::new(1.0, t.input_border), egui::StrokeKind::Outside);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.color_guide = options(app, d);
    app.ui.dialog = None;
    Ok(json!(app.ui.color_guide))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn ok_sets_the_panel_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("ui.colorGuideOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("steps".into(), json!(30));
        d.fields.insert("amount".into(), json!(80));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!((app.ui.color_guide.steps, app.ui.color_guide.amount), (20, 80.0), "steps are clamped to 20");
    }
}
