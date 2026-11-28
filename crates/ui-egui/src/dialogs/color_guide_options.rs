//! Color Guide Options: the variation steps on each side of the harmony colours (1–20) and how far
//! they reach (Less … More), with the base colour's variations as a preview. OK sets the Color
//! Guide panel's options (`UiState::color_guide`).
//!
//! Fields: `steps` (1–20) and `amount` (0–100).

use craft::imaging::ui::ToneRamp;
use craft::theme::Srgba;
use craft::ui::choice::SwatchTable;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::form::SliderField;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::style::DataColor;
use craft::ui::value_drag::TrackFace;
use vectorcraft_color::harmony::{GuideOptions, variation};

use super::swatch_options::{field_label, hosted};
use crate::VectorcraftApp;
use crate::panels::color_guide::base_color;

/// The dialog kind of Color Guide Options.
pub const KIND: &str = "colorGuideOptions";

/// Open the dialog with the panel's current options. Without a window to open on it says so in the status bar.
pub fn open(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else { return app.status(super::swatch_options::NO_WINDOW) };
    let current = app.ui.color_guide;
    // The base colour is read once: nothing changes it while the dialog is open.
    let base = base_color(app);
    let mut fields = DialogFields::new();
    fields.set("steps", f64::from(current.steps));
    fields.set("amount", f64::from(current.amount));
    let steps = Param::custom("steps", field_label(tl!("Steps:")), |ui, fields, _| {
        super::form::stacked(ui, fields, |ui, fields| {
            slider(ui, fields, "steps", tl!("Steps:"), 1.0..=f64::from(GuideOptions::MAX_STEPS), None);
            let ends = [(0.0, tl!("Less").to_owned()), (100.0, tl!("More").to_owned())];
            slider(ui, fields, "amount", tl!("Variation:"), 0.0..=100.0, Some(ends));
        })
    })
    .companions(["amount"]);
    let variations = Param::custom("variations", field_label(tl!("Variations:")), move |ui, fields, _| {
        let o = options(current, fields);
        let n = o.steps as i32;
        let chips: Vec<(Label, Srgba)> = (-n..=n)
            .map(|step| {
                let [r, g, b] = variation(base, &o, step).to_rgb();
                (Label::from(step.to_string()), Srgba::rgb(r, g, b))
            })
            .collect();
        ui.scope(|ui| {
            // The chips hold no selection: a click reports one the row ignores.
            let _ = SwatchTable::new(tl!("Variations").to_owned(), &chips).show_all(ui);
        })
        .response
    });
    let dialog = hosted(&ctx, KIND, tl!("Color Guide Options"), fields, vec![steps, variations], false);
    let ok = |app: &mut VectorcraftApp, answer: &ParamResult| -> Result<(), String> {
        app.ui.color_guide = options(app.ui.color_guide, &answer.fields);
        Ok(())
    };
    if super::host::open_held(app, dialog, ok).is_none() {
        app.status(super::swatch_options::NO_WINDOW);
    }
}

/// One slider with its number field over the ramp of greys, for the fields' `key`; `ends` are the words under its ends.
fn slider(
    ui: &mut egui::Ui,
    fields: &mut DialogFields,
    key: &str,
    label: &str,
    range: std::ops::RangeInclusive<f64>,
    ends: Option<[(f64, String); 2]>,
) {
    let ramp = [DataColor::new(egui::Color32::from_gray(60)), DataColor::new(egui::Color32::from_gray(220))];
    let shown = fields.number(key).unwrap_or(*range.start());
    let mut field = SliderField::new(field_label(label), Bind::given(shown), range).step(1.0).track_face(TrackFace::new(ToneRamp::new(&ramp)));
    if let Some(ends) = ends {
        field = field.marks_labelled(ends);
    }
    if let Some(v) = field.show(ui).edit.value() {
        fields.set(key, *v);
    }
}

/// The panel's options `current` with the dialog's steps and amount.
fn options(current: GuideOptions, fields: &DialogFields) -> GuideOptions {
    let steps = fields.number("steps").unwrap_or(f64::from(current.steps)).round().max(1.0) as u32;
    GuideOptions { steps, amount: fields.number("amount").unwrap_or(f64::from(current.amount)) as f32, ..current }.clamped()
}
