//! The options of PSD (colour model, a flat image or layers, Preserve Text Editability, Maximum
//! Editability, hidden layers, profile), after the raster export rows it shares with PNG Options ([`super::png_options`]): the
//! `psdOptions` dialog.

use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use serde_json::{Map, Value, json};
use vectorcraft_render::encode::jpeg::ColorModel;
use vectorcraft_render::encode::psd::PsdOptions;

use super::png_options::{check, choice};

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

/// The rows of PSD's own options. `layers` is a two-id radio, so the export sees a boolean.
pub(super) fn params() -> Vec<Param> {
    let layers = ParamKind::radio(vec![("false".into(), tl!("Flat Image").into()), ("true".into(), tl!("Write Layers").into())], Some("true".into()));
    vec![
        choice("colorModel", tl!("Color Model:"), &ColorModel::ALL.map(ColorModel::id), &ColorModel::ALL.map(ColorModel::label)),
        Param::new("layers", tl!("Options:"), layers),
        check("preserveText", tl!("Preserve Text Editability"), false).enabled_when("layers", "true"),
        check("maxEditability", tl!("Maximum Editability"), false).enabled_when("layers", "true"),
        check("hiddenLayers", tl!("Include Hidden Layers"), false).enabled_when("layers", "true"),
        check("embedIcc", tl!("Embed ICC Profile"), true),
    ]
}
