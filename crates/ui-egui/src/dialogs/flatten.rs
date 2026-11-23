//! Object → Flatten Transparency: a preset, the raster/vector balance, the resolutions and the
//! outline and preserve options, previewed live on the canvas (off at first: flattening complex
//! art takes a while); OK keeps the result as one undo step (`object.flattenTransparency`). The
//! presets row saves the options as a preset (`flattener.presets.save`).
//!
//! Fields: `preset` (a built-in or saved preset's name: setting it loads its options), the option
//! keys of [`FlattenOptions`] (`balance`, `lineArtPpi`, `gradientPpi`, `textToOutlines`,
//! `strokesToOutlines`, `clipComplexRegions`, `antiAlias`, `preserveAlpha`, `preserveOverprints`)
//! and `preview`.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::pdf;
use vectorcraft_engine::cmd::{FlattenOptions, FlattenerPreset};
use vectorcraft_pdf::Overprint;

use craft::ui::contract::{Bind, TooltipSpec};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, HostedPdfExport, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::patterns::presets::{NamedPresetEvent, PresetFns, PresetPicker, PresetRow, PresetSource};
use craft::ui::runtime::env::{Env, UnitKind};
use craft::ui::value_drag::SliderScale;

use super::form;
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use super::save_pdf::SavePdfJob;
use crate::VectorcraftApp;

/// The key of the Flatten Transparency dialog.
const KEY: &str = "flattenTransparency";

const CMD: &str = "object.flattenTransparency";

/// The switches the PDF 1.3 writer sets itself, which Custom Transparency Flattener Options show
/// but can't change: rasterized areas are always clipped to their regions and keep no alpha, and
/// overprints are kept only with Overprint set to Preserve.
pub(crate) const FIXED: [&str; 3] = ["clipComplexRegions", "preserveAlpha", "preserveOverprints"];

/// The `preserveOverprints` value Custom Transparency Flattener Options opened on, before
/// Overprint applies to it.
const KEPT_OVERPRINTS: &str = "__keptOverprints";

/// The key of Custom Transparency Flattener Options.
const CUSTOM_KEY: &str = "pdfFlattenerOptions";

/// The holder of the open Flatten Transparency: the live preview must outlive the frame that
/// opened the dialog.
#[derive(Default)]
pub(crate) struct FlattenDialog {
    open: Option<Open>,
}

/// An open Flatten Transparency.
pub(crate) struct Open {
    id: DialogId,
    preview: LivePreview,
}

/// Write `o` into the option fields.
pub(crate) fn put_options(fields: &mut Map<String, Value>, o: &FlattenOptions) {
    if let Value::Object(m) = serde_json::to_value(o).unwrap_or_default() {
        fields.extend(m);
    }
}

/// `o` as the values of the [`option_params`] rows.
pub(crate) fn option_fields(o: &FlattenOptions) -> DialogFields {
    let mut map = Map::new();
    put_options(&mut map, o);
    form::fields_of(&option_params(&[], ""), &map)
}

/// The flattener options as Flatten Transparency, the preset manager and the Flattener Preview
/// panel edit them: the raster/vector balance, both resolutions and the six switches. The switches
/// named in `fixed` show but can't be changed (the file format or another control sets them);
/// `reasons` is what their tooltip says.
pub(crate) fn option_params(fixed: &[&str], reasons: &'static str) -> Vec<Param> {
    let d = FlattenOptions::default();
    let ppi = |key, label, default| Param::number(key, label, default, 1.0, 0).bounds(Some(1.0), Some(2400.0)).unit(UnitKind::PixelsPerInch);
    let switches = [
        ("textToOutlines", tl!("Convert All Text to Outlines")),
        ("strokesToOutlines", tl!("Convert All Strokes to Outlines")),
        ("clipComplexRegions", tl!("Clip Complex Regions")),
        ("antiAlias", tl!("Anti-alias Rasters")),
        ("preserveAlpha", tl!("Preserve Alpha Transparency")),
        ("preserveOverprints", tl!("Preserve Overprints and Spot Colors")),
    ];
    let mut params = vec![
        Param::new("balance", tl!("Raster/Vector Balance:"), ParamKind::range(0.0, 100.0, d.balance, SliderScale::Linear))
            .marks_labelled([(0.0, tl!("Rasters")), (100.0, tl!("Vectors"))]),
        ppi("lineArtPpi", tl!("Line Art and Text Resolution:"), d.line_art_ppi),
        ppi("gradientPpi", tl!("Gradient and Mesh Resolution:"), d.gradient_ppi),
    ];
    params.extend(switches.into_iter().map(|(key, label)| {
        let row = Param::new(key, label, ParamKind::bool(false));
        if fixed.contains(&key) {
            row.enabled_when_in(key, std::iter::empty::<FieldValue>()).tooltip(TooltipSpec::new(reasons).disabled_text(reasons))
        } else {
            row
        }
    }));
    params
}

/// Is `name` a built-in flattener preset's?
pub(crate) fn is_builtin(name: &str) -> bool {
    FlattenOptions::PRESETS.iter().any(|id| FlattenOptions::preset_label(id) == Some(name))
}

/// The presets the session lists, as (name, shown name): a built-in preset's name is translated.
pub(crate) fn preset_names(app: &VectorcraftApp) -> Vec<(String, String)> {
    let shown = |name: &str| if is_builtin(name) { tl!(name).to_string() } else { name.to_string() };
    app.session.flattener_presets().iter().map(|p| (p.name.clone(), shown(&p.name))).collect()
}

/// The rows of (name, shown name) pairs.
fn name_rows(names: &&[(String, String)], _: &Env) -> Vec<PresetRow<String>> {
    names.iter().map(|(name, shown)| PresetRow::item(name.clone(), shown.clone(), 0)).collect()
}

/// The presets the session lists, as what a presets row takes: the rows (the built-in ones
/// translated), the values each applies, and the built-in names.
pub(crate) fn preset_rows(app: &VectorcraftApp) -> (Vec<PresetRow<String>>, Vec<(String, DialogFields)>, Vec<String>) {
    let presets = app.session.flattener_presets();
    let rows = preset_names(app).into_iter().map(|(name, shown)| PresetRow::item(name, shown, 0)).collect();
    let values = presets.iter().map(|p| (p.name.clone(), option_fields(&p.options))).collect();
    let built_in = presets.iter().filter(|p| is_builtin(&p.name)).map(|p| p.name.clone()).collect();
    (rows, values, built_in)
}

/// A picker over `presets` ([`preset_names`]) bound to the text field `key`, for a `Param::custom`
/// row. While `custom` is true no preset is current and the picker shows `[Custom]`.
pub(crate) fn preset_picker(ui: &mut egui::Ui, fields: &mut DialogFields, key: &str, presets: &[(String, String)], custom: bool) -> egui::Response {
    let current = if custom { None } else { fields.text(key).map(str::to_string) };
    let mut value = current.clone();
    let source = PresetSource::new(presets, PresetFns::new(name_rows));
    let out = PresetPicker::new(tl!("Preset:"), Bind::bound(&mut value), &source, Bind::kept()).placeholder(tl!("[Custom]")).show(ui);
    if value != current
        && let Some(name) = value
    {
        fields.set(key, name);
    }
    out.edit.response
}

/// `o` as the PDF 1.3 writer flattens with it (see [`FIXED`]).
pub(crate) fn as_written(o: &mut FlattenOptions, overprint: Overprint) {
    o.clip_complex_regions = true;
    o.preserve_alpha = false;
    o.preserve_overprints &= overprint != Overprint::Discard;
}

/// The name of the flattener preset Save PDF `parent` flattens with, as the list names it.
pub(super) fn preset_name(parent: &HostedPdfExport) -> String {
    let name = parent.settings().flattener_preset.trim();
    let name = if name.is_empty() { pdf::DEFAULT_FLATTENER } else { name };
    FlattenOptions::preset_label(&name.to_ascii_lowercase()).unwrap_or(name).to_string()
}

/// Custom Transparency Flattener Options over Save PDF `parent`, on the options it flattens with:
/// the job's own, else its preset's (`saved` lists them; the default preset's when it is gone).
// PLAN: `ctx` added to the contract signature: `HostedParamDialog::new` needs it.
pub(crate) fn custom_dialog(ctx: &egui::Context, parent: &HostedPdfExport, job: &SavePdfJob, saved: &[(String, DialogFields)]) -> HostedParamDialog {
    let params = option_params(&[], "");
    let name = preset_name(parent);
    let listed = || {
        let (_, fields) = saved.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name))?;
        FlattenOptions::from_params_with(&Value::Object(form::json_of(&params, fields)), &[]).ok()
    };
    let mut o = job.flattener.clone().or_else(listed).unwrap_or_else(|| FlattenOptions::preset(pdf::DEFAULT_FLATTENER).unwrap_or_default());
    let kept = o.preserve_overprints;
    as_written(&mut o, parent.settings().advanced.overprint);
    let mut fields = option_fields(&o);
    fields.set(KEPT_OVERPRINTS, kept);
    HostedParamDialog::new(ctx, CUSTOM_KEY, fields, |_| {
        ParamDialog::new(
            CUSTOM_KEY,
            tl!("Custom Transparency Flattener Options"),
            option_params(&FIXED, tl!("PDF 1.3 files clip rasterized areas to their regions, and Overprint sets whether overprints are kept.")),
        )
    })
}

/// OK of Custom Transparency Flattener Options: `job` flattens with these `fields` over its
/// preset (High Resolution when a saved preset was deleted meanwhile; then the result is `true`:
/// the caller blanks the parent's preset name). Options equal to the preset's leave the preset
/// alone; bad options are the refusal.
pub(crate) fn apply_custom(job: &mut SavePdfJob, parent: &HostedPdfExport, fields: &DialogFields, saved: &[FlattenerPreset]) -> Result<bool, String> {
    let mut o = FlattenOptions::from_params_with(&Value::Object(form::json_of(&option_params(&[], ""), fields)), &[])?;
    let name = preset_name(parent);
    let kept = fields.bool(KEPT_OVERPRINTS).or_else(|| {
        let custom = job.flattener.as_ref().and_then(|f| serde_json::to_value(f).ok());
        FlattenOptions::for_export(Some(&name), custom.as_ref(), saved).ok().map(|before| before.preserve_overprints)
    });
    let (base, deleted) = match FlattenOptions::find_preset(&name, saved) {
        Some(base) => (base, false),
        None => (FlattenOptions::preset(pdf::DEFAULT_FLATTENER).unwrap_or_default(), true),
    };
    // The writer sets the first two itself, so they take the preset's values: options that
    // otherwise match the preset leave its name showing. The writer keeps overprints only where the
    // options and Overprint both keep them, so the options keep the value they opened with, even
    // when their preset was deleted, changed or made again under its name meanwhile.
    o.clip_complex_regions = base.clip_complex_regions;
    o.preserve_alpha = base.preserve_alpha;
    o.preserve_overprints = kept.unwrap_or(o.preserve_overprints);
    job.flattener = (o != base).then_some(o);
    Ok(deleted)
}

/// Open Flatten Transparency for the selection with the default preset.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = HostedParamDialog::new(&ctx, KEY, option_fields(&FlattenOptions::default()), |_| {
        ParamDialog::new(KEY, tl!("Flatten Transparency"), option_params(&[], "")).previews(true)
    })
    // Flattening complex art is slow: the preview waits until it is turned on.
    .preview_off();
    let held = HeldOptions {
        parent: None,
        cancel: Some(Box::new(|app: &mut VectorcraftApp| {
            if let Some(open) = app.flatten_dialog.open.take() {
                open.preview.cancel(app);
            }
        })),
    };
    let id = open_held_with(app, dialog, ok, held).ok_or("no window")?;
    app.flatten_dialog.open = Some(Open { id, preview: LivePreview::default() });
    Ok(json!({ "dialog": KEY }))
}

/// The `object.flattenTransparency` parameters the fields hold.
fn command_params(params: &[Param], fields: &DialogFields, saved: &[FlattenerPreset]) -> Result<Value, String> {
    let o = FlattenOptions::from_params_with(&Value::Object(form::json_of(params, fields)), saved)?;
    let mut map = Map::new();
    put_options(&mut map, &o);
    Ok(Value::Object(map))
}

/// OK: flatten with the dialog's options, keeping the preview as the one undo step. A refusal
/// keeps the dialog open with its reason.
fn ok(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let params = command_params(&option_params(&[], ""), &result.fields, &app.session.prefs.flattener_presets)?;
    let preview = app.flatten_dialog.open.take().map(|o| o.preview).unwrap_or_default();
    preview.commit(app, CMD, params).map(|_| ()).inspect_err(|e| app.status(e.clone()))
}

/// Gives the dialog `id` the session's flattener presets.
fn list_presets(app: &VectorcraftApp, host: &mut craft::ui::dialog::DialogHost, id: DialogId) {
    let (rows, values, built_in) = preset_rows(app);
    if let Some(d) = host.get_mut_by_id::<HostedParamDialog>(id) {
        d.set_presets(rows, values, built_in);
    }
}

/// Saves `params` as the preset `name` and selects it in the dialog `id`; a refusal (a built-in
/// name) goes to the status bar.
fn create(app: &mut VectorcraftApp, ctx: &egui::Context, id: DialogId, params: Value, name: &str) {
    let Value::Object(mut p) = params else { return };
    p.insert("name".into(), json!(name));
    match app.run("flattener.presets.save", Value::Object(p)) {
        Ok(r) => {
            let saved = r["name"].as_str().unwrap_or(name).to_string();
            if let Some(dialogs) = app.dialogs() {
                dialogs.with(|host| {
                    list_presets(app, host, id);
                    // A dialog that closed meanwhile has nothing to select in.
                    let _ = host.set_json_field(ctx, id, "preset", json!(saved));
                });
            }
        }
        Err(e) => app.status(e),
    }
}

/// Once per frame: lists the presets, runs the live preview for the dialog's values and saves the
/// presets the user created.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(id) = app.flatten_dialog.open.as_ref().map(|o| o.id) else { return };
    let Some(dialogs) = app.dialogs() else { return };
    let params = option_params(&[], "");
    let seen = dialogs.with(|host| {
        list_presets(app, host, id);
        host.get_mut_by_id::<HostedParamDialog>(id).map(|d| (d.is_previewing(), d.fields().clone(), d.take_preset_events()))
    });
    let Some((previewing, fields, events)) = seen else { return };
    let options = command_params(&params, &fields, &app.session.prefs.flattener_presets);
    if let Some(mut open) = app.flatten_dialog.open.take() {
        match &options {
            Ok(p) => {
                // The answer is the preview's own; a refusal reaches the status bar inside `run`.
                let _ = open.preview.run(app, previewing, "Flatten Transparency", CMD, p);
            }
            // Invalid values keep the last preview; with the preview off there is nothing to keep.
            Err(_) if previewing => {}
            Err(_) => {
                let _ = open.preview.run(app, false, "Flatten Transparency", CMD, &Value::Null);
            }
        }
        app.flatten_dialog.open = Some(open);
    }
    for event in events {
        if let NamedPresetEvent::Create { name, .. } = event {
            match &options {
                Ok(p) => create(app, ctx, id, p.clone(), &name),
                Err(e) => app.status(e.clone()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// A frame of the whole app: the shell pass runs the dialog host, then the pump.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        crate::testing::app_frame(app, &ctx, egui::RawInput::default());
    }

    /// A document with a half-transparent red square over a blue one, both selected.
    fn scene() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
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

    fn open_dialog(app: &mut VectorcraftApp) -> DialogId {
        app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
        frame(app);
        app.flatten_dialog.open.as_ref().unwrap().id
    }

    fn open_scene() -> (VectorcraftApp, DialogId) {
        let mut app = scene();
        let id = open_dialog(&mut app);
        (app, id)
    }

    fn transparent_left(app: &VectorcraftApp) -> bool {
        let mut any = false;
        for l in &app.session.doc().unwrap().doc.layers {
            l.walk(&mut |n| any |= n.opacity < 1.0 && !n.is_container());
        }
        any
    }

    fn set(app: &VectorcraftApp, id: DialogId, field: &str, value: Value) {
        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, field, value)).unwrap();
    }

    fn number(app: &VectorcraftApp, id: DialogId, key: &str) -> f64 {
        app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().number(key))).unwrap()
    }

    fn confirm(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().unwrap();
        // A held OK answers in the frames after the press.
        app.dialogs().unwrap().with(|h| h.confirm::<HostedParamDialog>(&ctx));
        frame(app);
        frame(app);
    }

    #[test]
    fn cancel_rolls_the_preview_back_and_ok_flattens_as_one_step() {
        let (mut app, id) = open_scene();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        assert!(!app.session.in_interaction(), "no preview until it is turned on");
        set(&app, id, "preview", json!(true));
        frame(&mut app);
        assert!(app.session.in_interaction() && !transparent_left(&app), "previewed on the canvas");
        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|h| h.cancel::<HostedParamDialog>(&ctx));
        frame(&mut app);
        assert!(app.flatten_dialog.open.is_none() && !app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().doc, before, "Cancel rolls back");
        // OK without a preview flattens with the dialog's options as one undo step.
        let id = open_dialog(&mut app);
        set(&app, id, "preset", json!("High Resolution"));
        assert_eq!(number(&app, id, "lineArtPpi"), 1200.0, "a preset set from outside loads");
        set(&app, id, "balance", json!(0));
        confirm(&mut app);
        assert!(app.flatten_dialog.open.is_none());
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
        // A PNG whose header reads but whose pixel data is cut short.
        let img = craft_image_codecs::Image::from_raw(
            64,
            32,
            craft_image_codecs::ChannelLayout::Rgba,
            craft_image_codecs::SampleType::U8,
            [200u8, 30, 30, 255].repeat(64 * 32),
        )
        .unwrap();
        let png = craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
        for preview in [true, false] {
            let mut app = scene();
            let placed = vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({}));
            // The image is half transparent itself, so Flatten Transparency flattens it wherever it was placed.
            app.run("transparency.set", json!({"ids": placed["ids"], "opacity": 50})).unwrap();
            app.run("select.all", json!({})).unwrap();
            let id = open_dialog(&mut app);
            set(&app, id, "preview", json!(preview));
            frame(&mut app);
            assert_eq!(app.session.in_interaction(), preview);
            app.status("");
            confirm(&mut app);
            assert_eq!(app.ui.status, WARNING, "preview {preview}");
        }
    }

    #[test]
    fn save_preset_keeps_the_options_and_lists_them() {
        let (mut app, id) = open_scene();
        let ctx = app.ui_ctx.clone().unwrap();
        set(&app, id, "balance", json!(30));
        let fields = app.dialogs().unwrap().with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())).unwrap();
        let params = command_params(&option_params(&[], ""), &fields, &[]).unwrap();
        // The `Create` the presets row raises runs this.
        create(&mut app, &ctx, id, params.clone(), "Thirty");
        let saved = &app.session.prefs.flattener_presets;
        assert_eq!((saved.len(), saved[0].name.as_str(), saved[0].options.balance), (1, "Thirty", 30.0));
        set(&app, id, "preset", json!("Low Resolution"));
        assert_eq!(number(&app, id, "balance"), 75.0);
        set(&app, id, "preset", json!("Thirty"));
        assert_eq!(number(&app, id, "balance"), 30.0);
        // Built-in presets can't be replaced.
        create(&mut app, &ctx, id, params, "High Resolution");
        assert_eq!(app.session.prefs.flattener_presets.len(), 1);
    }
}
