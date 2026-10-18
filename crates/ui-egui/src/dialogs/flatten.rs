//! Object → Flatten Transparency: a preset, the raster/vector balance, the resolutions and the
//! outline and preserve options, previewed live on the canvas (off at first: flattening complex
//! art takes a while); OK keeps the result as one undo step (`object.flattenTransparency`). Save
//! Preset… keeps the options as a saved preset (`flattener.presets.save`).
//!
//! Fields: `preset` (a built-in or saved preset's name: setting it loads its options), the option
//! keys of [`FlattenOptions`] (`balance`, `lineArtPpi`, `gradientPpi`, `textToOutlines`,
//! `strokesToOutlines`, `clipComplexRegions`, `antiAlias`, `preserveAlpha`, `preserveOverprints`)
//! and `preview`.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::{FlattenOptions, FlattenerPreset};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Flatten Transparency.
pub const KIND: &str = "flattenTransparency";

const CMD: &str = "object.flattenTransparency";

/// The preset whose options the fields hold (a `preset` set from outside loads its options).
const APPLIED: &str = "__applied";
/// Save Preset… is open, with the name typed in `__presetName`.
const SAVING: &str = "__saving";
const PRESET_NAME: &str = "__presetName";

/// What the preset dropdown shows once the options differ from the preset's.
const CUSTOM: &str = "[Custom]";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Flatten Transparency").into(), body, confirm, preview: true, min_width: 420.0, ..DialogSpec::FORM };

/// Open Flatten Transparency for the selection with the default preset.
pub fn open(app: &mut VectorcraftApp) {
    let name = FlattenOptions::preset_label(FlattenOptions::PRESETS[1]).unwrap_or_default();
    let mut d = Dialog::new(KIND, json!({ "preset": name, APPLIED: name, "preview": false }));
    put_options(&mut d.fields, &FlattenOptions::default());
    app.ui.dialog = Some(d);
}

/// Write `o` into the option fields.
pub(crate) fn put_options(fields: &mut Map<String, Value>, o: &FlattenOptions) {
    if let Value::Object(m) = serde_json::to_value(o).unwrap_or_default() {
        fields.extend(m);
    }
}

/// The options the fields hold (a `preset` not loaded yet: its own).
fn options(d: &Dialog, saved: &[FlattenerPreset]) -> Result<FlattenOptions, String> {
    if d.str("preset") != d.str(APPLIED) {
        return FlattenOptions::from_params_with(&json!({ "preset": d.str("preset") }), saved);
    }
    FlattenOptions::from_params_with(&Value::Object(d.fields.clone()), saved)
}

/// `object.flattenTransparency` parameters: every option.
fn params(d: &Dialog, saved: &[FlattenerPreset]) -> Result<Value, String> {
    let mut fields = Map::new();
    put_options(&mut fields, &options(d, saved)?);
    Ok(Value::Object(fields))
}

/// Load the options of a preset set from outside (`ui.dialog.set`).
fn sync(d: &mut Dialog, saved: &[FlattenerPreset]) {
    let name = d.str("preset");
    if name != d.str(APPLIED) {
        if let Ok(o) = options(d, saved) {
            put_options(&mut d.fields, &o);
        }
        d.fields.insert(APPLIED.into(), json!(name));
    }
}

/// Make `name` the preset the fields hold.
fn applied(d: &mut Dialog, name: &str) {
    d.fields.insert("preset".into(), json!(name));
    d.fields.insert(APPLIED.into(), json!(name));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let saved = &app.session.prefs.flattener_presets;
    sync(d, saved);
    let current = options(d, saved);
    let presets = app.session.flattener_presets();
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Preset:"));
        if let Some(p) = preset_dropdown(ui, "flatten-preset", &presets, &d.str("preset"), current.as_ref().ok(), 220.0) {
            put_options(&mut d.fields, &p.options);
            applied(d, &p.name);
        }
        if widgets::flat_button(ui, tl!("Save Preset…"), 100.0).clicked() {
            let open = !d.bool(SAVING);
            d.fields.insert(SAVING.into(), json!(open));
            d.fields.insert(PRESET_NAME.into(), json!(app.session.new_preset_name()));
        }
    });
    if d.bool(SAVING) {
        save_row(app, ui, d);
    }
    ui.add_space(10.0);
    let mut o = current.clone().unwrap_or_default();
    if options_editor(ui, "flatten-dialog", &mut o, true) {
        put_options(&mut d.fields, &o);
    }
    if let Err(e) = &current {
        ui.label(egui::RichText::new(e).color(Tokens::get(ui.ctx()).text_dim).size(11.5));
    }
    // Invalid values keep the last preview.
    let p = params(d, &app.session.prefs.flattener_presets).unwrap_or_else(|_| d.fields.get(form::PREVIEWED).cloned().unwrap_or_default());
    form::preview(app, ui, d, "Flatten Transparency", CMD, p);
    false
}

/// Save Preset…: the new preset's name, Save and Cancel (Enter in the name saves).
fn save_row(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Name:"));
        let r = form::text_edit(ui, d, PRESET_NAME, 180.0);
        // Enter saves the preset instead of flattening.
        let enter = r.lost_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if (widgets::flat_button(ui, tl!("Save"), 60.0).clicked() || enter)
            && let Err(e) = save_preset(app, d)
        {
            app.status(e);
        }
        if widgets::flat_button(ui, tl!("Cancel"), 60.0).clicked() {
            d.fields.insert(SAVING.into(), json!(false));
        }
    });
    let name = d.str(PRESET_NAME);
    if app.session.prefs.flattener_presets.iter().any(|p| p.name.eq_ignore_ascii_case(name.trim())) {
        widgets::dim_label(ui, tl!("Replaces the saved preset of that name."));
    }
}

/// Keep the dialog's options as the saved preset `__presetName` and select it.
fn save_preset(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<(), String> {
    let mut p = params(d, &app.session.prefs.flattener_presets)?;
    p["name"] = json!(d.str(PRESET_NAME));
    let r = app.run("flattener.presets.save", p)?;
    applied(d, r["name"].as_str().unwrap_or_default());
    d.fields.insert(SAVING.into(), json!(false));
    Ok(())
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let p = params(d, &app.session.prefs.flattener_presets)?;
    form::commit_preview(app, CMD, p)
}

/// The flattener preset dropdown: `presets` by name, showing `name` while `options` are that
/// preset's, else [Custom]. The built-in presets are ours (translated); the saved ones are names.
/// Returns the preset chosen.
pub(crate) fn preset_dropdown<'a>(
    ui: &mut egui::Ui,
    id: &str,
    presets: &'a [FlattenerPreset],
    name: &str,
    options: Option<&FlattenOptions>,
    width: f32,
) -> Option<&'a FlattenerPreset> {
    let shown = presets.iter().find(|p| p.name == name && Some(&p.options) == options).map_or(tl!(CUSTOM), |p| p.name.as_str());
    let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
    super::mixed_dropdown(ui, id, shown, &names, width, |k| names.get(k).is_some_and(|n| is_builtin(n))).and_then(|i| presets.get(i))
}

/// Is `name` a built-in flattener preset's?
fn is_builtin(name: &str) -> bool {
    FlattenOptions::PRESETS.iter().any(|id| FlattenOptions::preset_label(id) == Some(name))
}

/// The width of the option value fields.
const FIELD: f32 = 56.0;

/// The flattener options as Flatten Transparency, the preset manager and the Flattener Preview
/// panel edit them: the raster/vector balance, both resolutions and the six switches. Returns true
/// when one changed.
pub(crate) fn options_editor(ui: &mut egui::Ui, id: &str, o: &mut FlattenOptions, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let before = o.clone();
    let dim = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(tl!(s)).color(t.text_dim).size(11.5));
    };
    ui.add_enabled_ui(enabled, |ui| {
        widgets::dim_label(ui, tl!("Raster/Vector Balance:"));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            dim(ui, tl!("Rasters"));
            let rail = (ui.available_width() - FIELD - 56.0).clamp(80.0, 200.0);
            let shown = widgets::SliderValue::field(o.balance, 0.0..=100.0, "", 0);
            if let (Some(x), _) = widgets::color_slider_valued(
                ui,
                (id, "balance-slider"),
                tl!("Raster/Vector Balance:"),
                Some(shown),
                (o.balance / 100.0) as f32,
                rail,
                &|_| t.input_border,
            ) {
                o.balance = (x as f64 * 100.0).round();
            }
            dim(ui, tl!("Vectors"));
            if let Some(v) = widgets::plain_field(ui, (id, "balance"), o.balance, "", 0, FIELD - 12.0) {
                o.balance = v.round().clamp(0.0, 100.0);
            }
        });
        ui.add_space(4.0);
        egui::Grid::new((id, "resolutions")).num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (label, key) in [(tl!("Line Art and Text Resolution:"), 0), (tl!("Gradient and Mesh Resolution:"), 1)] {
                widgets::dim_label(ui, label);
                ui.horizontal(|ui| {
                    let v = if key == 0 { &mut o.line_art_ppi } else { &mut o.gradient_ppi };
                    if let Some(n) = widgets::plain_field(ui, (id, key), *v, "", 0, FIELD) {
                        *v = n.round().clamp(1.0, 2400.0);
                    }
                    dim(ui, "ppi");
                });
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        let switches: [(&str, &mut bool); 6] = [
            (tl!("Convert All Text to Outlines"), &mut o.text_to_outlines),
            (tl!("Convert All Strokes to Outlines"), &mut o.strokes_to_outlines),
            (tl!("Clip Complex Regions"), &mut o.clip_complex_regions),
            (tl!("Anti-alias Rasters"), &mut o.anti_alias),
            (tl!("Preserve Alpha Transparency"), &mut o.preserve_alpha),
            (tl!("Preserve Overprints and Spot Colors"), &mut o.preserve_overprints),
        ];
        for (label, v) in switches {
            if widgets::check(ui, label, *v, enabled) {
                *v = !*v;
            }
        }
    });
    *o != before
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// The built-in presets are told from saved ones by name (only theirs are translated).
    #[test]
    fn built_in_presets_are_known_by_name() {
        assert!(FlattenOptions::builtin_presets().iter().all(|p| is_builtin(&p.name)));
        assert!(!is_builtin("Regular") && !is_builtin("high"));
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    /// A document with a half-transparent red square over a blue one, both selected.
    fn scene() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        for (x, color) in [(10, "#0000ff"), (60, "#ff0000")] {
            let id = app.run("shape.rectangle", json!({"x": x, "y": x, "width": 100, "height": 100})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"color": color, "ids": [id]})).unwrap();
            app.run("paint.setStroke", json!({"none": true, "ids": [id]})).unwrap();
        }
        app.run("transparency.set", json!({"opacity": 50})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app
    }

    fn transparent_left(app: &VectorcraftApp) -> bool {
        let mut any = false;
        for l in &app.session.doc().unwrap().doc.layers {
            l.walk(&mut |n| any |= n.opacity < 1.0 && !n.is_container());
        }
        any
    }

    #[test]
    fn cancel_rolls_the_preview_back_and_ok_flattens_as_one_step() {
        let mut app = scene();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        frame(&mut app);
        assert!(!app.session.in_interaction(), "no preview until it is turned on");
        app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(true));
        frame(&mut app);
        assert!(app.session.in_interaction() && !transparent_left(&app), "previewed on the canvas");
        super::super::cancel(&mut app);
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().doc, before, "Cancel rolls back");
        // OK without a preview flattens with the dialog's options as one undo step.
        app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("preset".into(), json!("High Resolution"));
        frame(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("lineArtPpi", 0.0), 1200.0, "a preset set from outside loads");
        app.ui.dialog.as_mut().unwrap().fields.insert("balance".into(), json!(0));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let mut images = 0;
        st.doc.layers[0].walk(&mut |n| images += matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)) as usize);
        assert_eq!(images, 1, "balance 0 rasterizes");
        assert!(!transparent_left(&app));
    }

    #[test]
    fn ok_says_in_the_status_bar_when_an_image_could_not_be_decoded() {
        const WARNING: &str = "an image couldn't be decoded and is missing from the picture";
        // A PNG whose header reads (so it places at its size) but whose pixel data is cut short.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        png.truncate(idat + 8);
        for preview in [true, false] {
            let mut app = scene();
            let placed = app.run("file.place", json!({"name": "cut.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
            // The image is half transparent itself, so Flatten Transparency flattens it wherever it was placed.
            app.run("transparency.set", json!({"ids": placed["ids"], "opacity": 50})).unwrap();
            app.run("select.all", json!({})).unwrap();
            app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(preview));
            frame(&mut app);
            assert_eq!(app.session.in_interaction(), preview);
            app.status("");
            super::super::confirm(&mut app).unwrap();
            assert_eq!(app.ui.status, WARNING, "preview {preview}");
        }
    }

    #[test]
    fn save_preset_keeps_the_options_and_lists_them() {
        let mut app = scene();
        app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert("balance".into(), json!(30));
        d.fields.insert(PRESET_NAME.into(), json!("Thirty"));
        save_preset(&mut app, &mut d).unwrap();
        assert_eq!(d.str("preset"), "Thirty");
        let saved = &app.session.prefs.flattener_presets;
        assert_eq!((saved.len(), saved[0].name.as_str(), saved[0].options.balance), (1, "Thirty", 30.0));
        // A saved preset set from outside loads like a built-in one.
        d.fields.insert("preset".into(), json!("Low Resolution"));
        app.ui.dialog = Some(d);
        frame(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("balance", 0.0), 75.0);
        app.ui.dialog.as_mut().unwrap().fields.insert("preset".into(), json!("Thirty"));
        frame(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("balance", 0.0), 30.0);
        // Built-in presets can't be replaced.
        let mut d = app.ui.dialog.clone().unwrap();
        d.fields.insert(PRESET_NAME.into(), json!("High Resolution"));
        assert!(save_preset(&mut app, &mut d).is_err());
    }

    #[test]
    fn bad_values_report_and_keep_the_dialog_open() {
        let mut app = scene();
        app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("gradientPpi".into(), json!(9000));
        frame(&mut app);
        assert!(super::super::confirm(&mut app).unwrap_err().contains("resolutions"));
        assert!(app.ui.dialog.is_some());
    }
}
