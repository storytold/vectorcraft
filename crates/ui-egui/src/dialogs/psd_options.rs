//! The options of PSD (colour model, a flat image or layers, Preserve Text Editability, Maximum
//! Editability, hidden layers, profile), after the raster export rows it shares with PNG Options ([`super::png_options`]): the
//! `psdOptions` dialog.

use serde_json::{Map, Value, json};
use vectorcraft_render::encode::jpeg::ColorModel;
use vectorcraft_render::encode::psd::PsdOptions;

use super::form;
use super::png_options::choice;
use crate::state::Dialog;

/// The options PSD starts with (`cmyk`: a CMYK document exports CMYK).
pub(super) fn defaults(cmyk: bool, o: &mut Map<String, Value>) {
    let p = PsdOptions::default();
    let model = if cmyk { ColorModel::Cmyk } else { p.color_model };
    o.extend([
        ("colorModel".into(), json!(model.id())),
        ("layers".into(), json!(p.layers)),
        ("preserveText".into(), json!(p.preserve_text)),
        ("maxEditability".into(), json!(p.max_editability)),
        ("hiddenLayers".into(), json!(p.hidden_layers)),
        ("embedIcc".into(), json!(p.embed_icc)),
    ]);
}

/// Flat Image writes no layers: the options that need them are left out of the export's params
/// (`preserveText`, which the export refuses without layers).
pub(super) fn drop_unused(p: &mut Value) {
    if p["layers"] == json!(false)
        && let Some(o) = p.as_object_mut()
    {
        o.remove("preserveText");
    }
}

/// The grid rows of PSD's own options.
pub(super) fn rows(ui: &mut egui::Ui, d: &mut Dialog, label: &dyn Fn(&mut egui::Ui, &str) -> egui::Response) {
    label(ui, tl!("Color Model:"));
    choice(ui, d, "colorModel", &ColorModel::ALL.map(ColorModel::id), &ColorModel::ALL.map(ColorModel::label));
    ui.end_row();

    label(ui, tl!("Options:"));
    ui.vertical(|ui| {
        let mut layers = d.bool("layers");
        if ui.radio_value(&mut layers, false, tl!("Flat Image")).changed() | ui.radio_value(&mut layers, true, tl!("Write Layers")).changed() {
            d.fields.insert("layers".into(), json!(layers));
        }
        ui.add_enabled_ui(layers, |ui| {
            for (key, text) in [
                ("preserveText", tl!("Preserve Text Editability")),
                ("maxEditability", tl!("Maximum Editability")),
                ("hiddenLayers", tl!("Include Hidden Layers")),
            ] {
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    form::check(ui, d, key, text);
                });
            }
        });
    });
    ui.end_row();

    ui.label("");
    form::check(ui, d, "embedIcc", tl!("Embed ICC Profile"));
    ui.end_row();
}
