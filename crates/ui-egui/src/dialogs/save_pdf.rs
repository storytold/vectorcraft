//! File → Save as PDF: the Save PDF dialog. Preset, Standard and Compatibility on top, then the
//! sections General, Compression, Marks and Bleeds, Output, Advanced, Security and Summary.
//!
//! The fields are the `document.exportPdf` options (sections are objects such as `compression`),
//! plus `preset`, `range`, `path?` and UI-only `__` keys, so agents fill the dialog with
//! `ui.dialog.set`. OK runs `document.exportPdf` through [`io::export_pdf`]; the Summary is
//! `document.pdfSettings`. Save Preset… names the settings as a preset (`pdf.preset.save`).
//!
//! The same dialog edits a preset (kind `pdfPreset`, from Edit → PDF Presets): a `name` and
//! `description` instead of the preset and artboards, and OK saves the preset.

use std::sync::LazyLock;

use serde_json::{Map, Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::fileio::{SaveMode, pdf};
use vectorcraft_pdf::{
    Changes, Choice, ColorConversion, Compatibility, Downsample, Encryption, ImageCodec, JpegQuality, MarkKind, MonoCodec, Overprint, PdfSettings,
    Printing, ProfileInclusion, Standard,
};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "savePdf";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Save PDF").into(), body, confirm, ok: Some("Save PDF"), inputs: |_| &["path"], ..DialogSpec::FORM };

/// The dialog kind of the preset editor (New / Edit in Edit → PDF Presets).
pub(super) const PRESET_KIND: &str = "pdfPreset";

pub(super) const PRESET_SPEC: DialogSpec = DialogSpec {
    heading: |d| if d.str(EDITING).is_empty() { tl!("New PDF Preset") } else { tl!("Edit PDF Preset") }.into(),
    body,
    confirm: confirm_preset,
    ok: Some("Save Preset"),
    ..DialogSpec::FORM
};

/// The preset editor's field holding the name of the saved preset it edits (empty: a new one).
const EDITING: &str = "__editing";
/// Save PDF's field holding the name typed for Save Preset… (present while that row shows).
const SAVE_AS: &str = "__savePresetAs";

pub(super) const SECTIONS: [&str; 7] = ["General", "Compression", "Marks and Bleeds", "Output", "Advanced", "Security", "Summary"];

/// The default settings as JSON: what a field an agent left out reads as.
static DEFAULTS: LazyLock<Value> = LazyLock::new(|| serde_json::to_value(PdfSettings::default()).unwrap_or_default());

/// Is `name` a built-in PDF preset's? Those are ours (translated where listed); the saved ones are
/// names.
pub(super) fn is_builtin_preset(name: &str) -> bool {
    static NAMES: LazyLock<Vec<String>> = LazyLock::new(|| vectorcraft_pdf::builtin_presets().into_iter().map(|p| p.name).collect());
    NAMES.iter().any(|n| n == name)
}

/// Trim mark weights offered (pt).
const WEIGHTS: [f64; 3] = [0.125, 0.25, 0.5];
const WEIGHT_LABELS: [&str; 3] = ["0.125 pt", "0.25 pt", "0.5 pt"];

/// The Destination entry for the document's own profile (stored as "").
const DOCUMENT_PROFILE: &str = "Document profile";
/// The Output Intent Profile entry for no output intent (stored as "").
const NO_PROFILE: &str = "None";
/// That entry with a PDF/X standard, whose files always have an output intent.
const PDFX_INTENT: &str = "CMYK profile in effect";

pub(super) const LABEL_WIDTH: f32 = 150.0;
/// Width of the section list (its frame adds 6 px a side).
pub(super) const LIST_WIDTH: f32 = 150.0;
/// The top rows' labels end where the section content starts, so their dropdowns line up with it.
pub(super) const TOP_LABEL_WIDTH: f32 = LIST_WIDTH + 26.0;

/// Open the dialog with `params` (`document.exportPdf` options and `path?`) applied over their
/// preset.
pub fn open(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let mut fields = settings_fields(app, params)?;
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    fields.insert("preset".into(), json!(s("preset").unwrap_or(pdf::DEFAULT_PRESET)));
    fields.insert("__presets".into(), json!(pdf::presets(&app.session)));
    fields.insert("__section".into(), json!(SECTIONS[0]));
    fields.insert("__allArtboards".into(), json!(s("range").is_none()));
    fields.insert("range".into(), json!(s("range").unwrap_or("")));
    if let Some(path) = s("path") {
        fields.insert("path".into(), json!(path));
    }
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(Value::Null)
}

/// The settings `params` ask for (their preset, built-in or saved, with their options applied) as
/// dialog fields. A standard the writer can't produce yet still shows (Save PDF then says so).
fn settings_fields(app: &VectorcraftApp, params: &Value) -> Result<Map<String, Value>, String> {
    let settings = pdf::resolve("ui.savePdfDialog", params, &app.session.prefs.pdf_presets).map_err(|e| e.to_string())?;
    match serde_json::to_value(&settings).map_err(|e| e.to_string())? {
        Value::Object(mut fields) => {
            // Serialization omits passwords from presets and summaries; the live dialog needs
            // the validated passwords so confirming it still encrypts the PDF.
            let security = fields.get_mut("security").and_then(Value::as_object_mut).ok_or("PDF security settings are not an object")?;
            for (key, password) in
                [("openPassword", settings.security.open_password), ("permissionsPassword", settings.security.permissions_password)]
            {
                if !password.is_empty() {
                    security.insert(key.into(), json!(password));
                }
            }
            Ok(fields)
        }
        _ => Err("PDF settings are not an object".into()),
    }
}

/// The `document.exportPdf` params the dialog stands for.
fn params(d: &Dialog) -> Value {
    let mut p = form::params(d);
    if (d.bool("__allArtboards") || d.str("range").trim().is_empty())
        && let Some(o) = p.as_object_mut()
    {
        o.remove("range");
    }
    p
}

/// Save PDF: write the file (a PDF copy, or the PDF a Save As / Save a Copy asked for, `__save`);
/// the dialog stays open when that fails (bad range, cancelled save…). While Save Preset… asks for
/// a name, OK (and Enter) saves the preset instead.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.fields.contains_key(SAVE_AS) {
        let mut d = d.clone();
        let r = save_preset(app, &mut d);
        app.ui.dialog = Some(d);
        return r;
    }
    let r = match SaveMode::of(&d.str("__save")) {
        Some(mode) => io::save_pdf(app, mode, params(d)),
        None => io::export_pdf(app, params(d)),
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

/// The settings as `pdf.preset.save` params (without the artboards and path).
fn preset_params(d: &Dialog) -> Value {
    let mut p = form::params(d);
    if let Some(o) = p.as_object_mut() {
        for k in ["range", "path"] {
            o.remove(k);
        }
    }
    p
}

/// Save Preset…: save the settings under the typed name and pick that preset.
fn save_preset(app: &mut VectorcraftApp, d: &mut Dialog) -> Result<Value, String> {
    let mut p = preset_params(d);
    p["name"] = json!(d.str(SAVE_AS));
    let r = app.run("pdf.preset.save", p)?;
    d.fields.remove(SAVE_AS);
    d.fields.insert("preset".into(), r["name"].clone());
    d.fields.insert("__presets".into(), json!(pdf::presets(&app.session)));
    Ok(r)
}

/// Open the preset editor on the saved preset `name`, or on a new preset starting from `preset`
/// (default: the app default).
pub fn open_preset(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    let (base, name, description, editing) = match s("name") {
        Some(name) => {
            let p = app.session.prefs.pdf_presets.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()));
            let p = p.ok_or_else(|| format!("no saved PDF preset named `{name}` (built-in presets are read-only: start a new one from them)"))?;
            (p.name.clone(), p.name.clone(), p.description.clone(), p.name.clone())
        }
        None => (s("preset").unwrap_or(pdf::DEFAULT_PRESET).to_string(), app.session.new_pdf_preset_name(), String::new(), String::new()),
    };
    let mut fields = settings_fields(app, &json!({ "preset": base }))?;
    fields.insert("name".into(), json!(name));
    fields.insert("description".into(), json!(description));
    fields.insert(EDITING.into(), json!(editing));
    fields.insert("__section".into(), json!(SECTIONS[0]));
    app.ui.dialog = Some(Dialog { kind: PRESET_KIND.into(), fields });
    Ok(Value::Null)
}

/// Save Preset (the preset editor's OK): `pdf.preset.save`, then back to Edit → PDF Presets on it.
/// A new preset may not take the name of another one.
fn confirm_preset(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = preset_params(d);
    let (name, editing) = (d.str("name"), d.str(EDITING));
    if editing.is_empty() {
        if app.session.pdf_presets().iter().any(|q| q.name.eq_ignore_ascii_case(name.trim())) {
            return Err(format!("a preset named `{}` exists", name.trim()));
        }
    } else {
        p["name"] = json!(editing);
        p["newName"] = json!(name);
    }
    let r = app.run("pdf.preset.save", p)?;
    super::pdf_presets::open(app, r["name"].as_str());
    Ok(r)
}

// ---------- fields by path (`compression.color.ppi`) ----------

fn lookup<'a>(m: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut keys = path.split('.');
    let first = m.get(keys.next()?)?;
    keys.try_fold(first, |v, k| v.get(k))
}

/// The value at `path`: the dialog's, else the default of the settings it edits (the PDF
/// settings, or the print settings of the Print dialogs).
pub(super) fn get<'a>(d: &'a Dialog, path: &str) -> &'a Value {
    let defaults = super::print::defaults(&d.kind).unwrap_or(&DEFAULTS);
    lookup(&d.fields, path).or_else(|| defaults.as_object().and_then(|m| lookup(m, path))).unwrap_or(&Value::Null)
}

pub(super) fn set(d: &mut Dialog, path: &str, value: Value) {
    let (parents, key) = path.rsplit_once('.').unwrap_or(("", path));
    let mut map = &mut d.fields;
    for k in parents.split('.').filter(|k| !k.is_empty()) {
        let e = map.entry(k).or_insert(Value::Null);
        if !e.is_object() {
            *e = Value::Object(Map::new());
        }
        let Value::Object(m) = e else { return };
        map = m;
    }
    map.insert(key.into(), value);
}

// ---------- widgets bound to a path ----------

pub(super) fn heading(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(text).font(theme::semibold(12.5)).color(t.text_strong));
    ui.add_space(2.0);
}

pub(super) fn note(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(text).size(11.5).color(t.text_dim));
}

/// A labelled row.
pub(super) fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    row_with(ui, label, LABEL_WIDTH, add);
}

pub(super) fn row_with(ui: &mut egui::Ui, label: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        let label_id = ui
            .allocate_ui_with_layout(egui::vec2(width, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(width);
                ui.label(egui::RichText::new(label).color(t.text)).id
            })
            .inner;
        ui.label_inputs_by(label_id, add);
    });
}

pub(super) fn flag(ui: &mut egui::Ui, d: &mut Dialog, path: &str, label: &str, enabled: bool) {
    let on = get(d, path).as_bool().unwrap_or(false);
    if widgets::check(ui, label, on, enabled) {
        set(d, path, json!(!on));
    }
}

/// The index in `T`'s choices of the value at `path`.
fn choice_index<T: Choice>(d: &Dialog, path: &str) -> Option<usize> {
    get(d, path).as_str().and_then(|s| T::IDS.iter().position(|i| *i == s))
}

/// The choice at `path`.
pub(super) fn choice<T: Choice>(d: &Dialog, path: &str) -> Option<T> {
    choice_index::<T>(d, path).and_then(|i| T::ALL.get(i).copied())
}

/// A dropdown of the choices `T` (options for which `enabled` is false are greyed).
pub(super) fn pick<T: Choice>(ui: &mut egui::Ui, d: &mut Dialog, path: &str, width: f32, enabled: impl Fn(T) -> bool) -> bool {
    let current = choice_index::<T>(d, path);
    let labels: Vec<_> = T::LABELS.iter().map(|label| choice_label(crate::i18n::current(), path, label)).collect();
    let label = current.and_then(|i| labels.get(i)).copied().unwrap_or_default();
    let chosen = widgets::dropdown_with(ui, path, label, &labels, width, |i| T::ALL.get(i).is_some_and(|c| enabled(*c)));
    let Some(id) = chosen.and_then(|i| T::IDS.get(i)) else { return false };
    set(d, path, json!(id));
    true
}

/// Printer mark styles use adjectives, separately from interface language names.
pub(super) fn choice_label<'a>(lang: crate::i18n::Lang, path: &str, label: &'a str) -> &'a str {
    if path == "marks.kind" { crate::i18n::tr_ctx(lang, "mark-style", label) } else { crate::i18n::tr(lang, label) }
}

pub(super) fn number(ui: &mut egui::Ui, d: &mut Dialog, path: &str, suffix: &str, enabled: bool) {
    let v = get(d, path).as_f64().unwrap_or(0.0);
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(x) = widgets::plain_field(ui, path, v, suffix, 3, 64.0) {
            set(d, path, json!(x));
        }
    });
}

/// A length in points, shown in the document's units.
pub(super) fn length(ui: &mut egui::Ui, d: &mut Dialog, path: &str, unit: Unit, enabled: bool) {
    let v = get(d, path).as_f64();
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(x) = widgets::num_field(ui, path, v, unit, 80.0) {
            set(d, path, json!(x));
        }
    });
}

pub(super) fn text(ui: &mut egui::Ui, d: &mut Dialog, path: &str, enabled: bool) {
    let mut s = get(d, path).as_str().unwrap_or_default().to_string();
    if ui.add_enabled(enabled, egui::TextEdit::singleline(&mut s).align(egui::Align2::LEFT_CENTER).desired_width(240.0)).changed() {
        set(d, path, json!(s));
    }
}

// ---------- the dialog ----------

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let editor = d.kind == PRESET_KIND;
    if editor {
        row_with(ui, tl!("Preset name:"), TOP_LABEL_WIDTH, |ui| {
            form::text_edit(ui, d, "name", 288.0);
        });
    } else {
        preset_rows(app, ui, d);
    }
    row_with(ui, tl!("Standard:"), TOP_LABEL_WIDTH, |ui| {
        let picked = pick::<Standard>(ui, d, "standard", 170.0, |_| true);
        let standard = choice::<Standard>(d, "standard").unwrap_or_default();
        if picked {
            standard_chosen(d, standard);
        }
        ui.add_space(16.0);
        ui.label(egui::RichText::new(tl!("Compatibility:")).color(t.text));
        pick::<Compatibility>(ui, d, "compatibility", 110.0, |c| standard.allows(c));
    });
    ui.add_space(10.0);
    let section = current_section(d, &SECTIONS);
    ui.horizontal_top(|ui| {
        section_frame(ui, |ui| section_list(ui, d, &SECTIONS, section));
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.set_width(500.0);
            ui.label(egui::RichText::new(tl!(section)).font(theme::semibold(14.0)).color(t.text_strong));
            egui::ScrollArea::vertical().id_salt(("save-pdf", section)).max_height(360.0).auto_shrink([false, false]).show(ui, |ui| match section {
                "Compression" => compression(ui, d),
                "Marks and Bleeds" => marks_and_bleeds(app, ui, d),
                "Output" => output(ui, d),
                "Advanced" => advanced(app, ui, d),
                "Security" => security(ui, d),
                "Summary" => summary(app, ui, d),
                _ => general(ui, d, editor),
            });
        });
    });
    false
}

/// The section `__section` names (default: the first).
pub(super) fn current_section(d: &Dialog, sections: &[&'static str]) -> &'static str {
    sections.iter().copied().find(|s| *s == d.str("__section")).or_else(|| sections.first().copied()).unwrap_or_default()
}

/// The frame of the section list column, at least as tall as the section content.
pub(super) fn section_frame(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.fill(t.panel_darker).corner_radius(egui::CornerRadius::same(4)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_width(LIST_WIDTH);
        ui.set_min_height(380.0);
        ui.vertical(add);
    });
}

/// The section list: clicking a section shows it (`__section`).
pub(super) fn section_list(ui: &mut egui::Ui, d: &mut Dialog, sections: &[&str], section: &str) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 1.0;
    for s in sections {
        let sel = *s == section;
        let label = egui::RichText::new(tl!(*s)).size(12.5).color(if sel { t.text_strong } else { t.text });
        if ui.add(egui::Button::selectable(sel, label).min_size(egui::vec2(LIST_WIDTH - 12.0, 24.0))).clicked() {
            d.fields.insert("__section".into(), json!(s));
        }
    }
}

/// What choosing `standard` changes: the compatibility, to its version unless it allows the one
/// chosen, and off what files of a standard can't have (editing data, passwords; layers in PDF/X-1a
/// and PDF/X-3).
fn standard_chosen(d: &mut Dialog, standard: Standard) {
    if choice::<Compatibility>(d, "compatibility").is_some_and(|c| !standard.allows(c)) {
        set(d, "compatibility", json!(standard.version().id()));
    }
    if standard != Standard::None {
        set(d, "preserveEditing", json!(false));
        for p in [OPEN_PASSWORD, PERMISSIONS_PASSWORD] {
            set(d, p, json!(""));
        }
    }
    if !standard.allows_layers() {
        set(d, "createLayers", json!(false));
    }
}

/// The Preset row (the presets, built-in and saved, and Save Preset…) and, while Save Preset…
/// asks for a name, the row taking it.
fn preset_rows(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    row_with(ui, tl!("Preset:"), TOP_LABEL_WIDTH, |ui| {
        let presets = pdf::presets(&app.session);
        let names: Vec<&str> = presets.iter().map(String::as_str).collect();
        // The built-in presets come first (translated); the saved ones are names.
        let builtins = names.len().saturating_sub(app.session.prefs.pdf_presets.len());
        let chosen = super::mixed_dropdown(ui, "pdf-preset", &d.str("preset"), &names, 300.0, |k| k < builtins)
            .and_then(|i| names.get(i))
            .map(|p| p.to_string());
        if let Some(name) = chosen {
            apply_preset(app, d, &name);
        }
        let asking = d.fields.contains_key(SAVE_AS);
        if ui
            .add_enabled_ui(!asking, |ui| widgets::flat_button(ui, tl!("Save Preset…"), 96.0))
            .inner
            .on_hover_text(tl!("Save these settings as a preset"))
            .clicked()
        {
            d.fields.insert(SAVE_AS.into(), json!(app.session.new_pdf_preset_name()));
        }
    });
    if d.fields.contains_key(SAVE_AS) {
        row_with(ui, tl!("Save as preset:"), TOP_LABEL_WIDTH, |ui| {
            form::text_edit(ui, d, SAVE_AS, 200.0);
            if widgets::flat_button(ui, tl!("Save"), 52.0).clicked()
                && let Err(e) = save_preset(app, d)
            {
                app.status(e);
            }
            if widgets::flat_button(ui, tl!("Cancel"), 60.0).clicked() {
                d.fields.remove(SAVE_AS);
            }
        });
    }
}

/// Replace the settings with preset `name`'s (the artboard choice and path stay).
fn apply_preset(app: &mut VectorcraftApp, d: &mut Dialog, name: &str) {
    match settings_fields(app, &json!({ "preset": name })) {
        Ok(settings) => {
            d.fields.extend(settings);
            d.fields.insert("preset".into(), json!(name));
        }
        Err(e) => app.status(e),
    }
}

/// General: the options, then the artboards (Save PDF) or the description (preset editor).
fn general(ui: &mut egui::Ui, d: &mut Dialog, editor: bool) {
    heading(ui, tl!("Options"));
    let standard = choice::<Standard>(d, "standard").unwrap_or_default();
    flag(ui, d, "preserveEditing", tl!("Preserve editing capabilities"), standard == Standard::None);
    flag(ui, d, "thumbnails", tl!("Embed page thumbnails"), true);
    flag(ui, d, "fastWebView", tl!("Optimize for fast web view"), true);
    flag(ui, d, "viewAfterSaving", tl!("View PDF after saving"), true);
    // PDF layers need PDF 1.5, and PDF/X-1a and PDF/X-3 have none.
    let layers = standard.allows_layers() && choice::<Compatibility>(d, "compatibility").unwrap_or_default().has_layers();
    flag(ui, d, "createLayers", tl!("Create PDF layers from layers and sublayers"), layers);
    flag(ui, d, "includeNonPrinting", tl!("Include non-printing layers"), true);
    if editor {
        heading(ui, tl!("Description"));
        if let Some(text) = widgets::text_field_named(ui, "pdf-preset-description", Some(tl!("Description")), Some(&d.str("description")), 460.0, 3) {
            d.fields.insert("description".into(), json!(text));
        }
        return;
    }
    heading(ui, tl!("Artboards"));
    let mut all = d.bool("__allArtboards");
    ui.horizontal(|ui| {
        let changed = ui.radio_value(&mut all, true, tl!("All")).changed() | ui.radio_value(&mut all, false, tl!("Range:")).changed();
        if changed {
            d.fields.insert("__allArtboards".into(), json!(all));
        }
        let mut range = d.str("range");
        if ui
            .add_enabled(!all, egui::TextEdit::singleline(&mut range).align(egui::Align2::LEFT_CENTER).desired_width(140.0).hint_text("1-3, 5"))
            .changed()
        {
            d.fields.insert("range".into(), json!(range));
        }
    });
}

fn image_rows(ui: &mut egui::Ui, d: &mut Dialog, key: &str, title: &str) {
    heading(ui, title);
    let base = format!("compression.{key}");
    let on = get(d, &format!("{base}.downsample")) != Downsample::None.id();
    ui.horizontal(|ui| {
        pick::<Downsample>(ui, d, &format!("{base}.downsample"), 150.0, |_| true);
        ui.label(tl!("to"));
        number(ui, d, &format!("{base}.ppi"), " ppi", on);
        ui.label(tl!("above"));
        number(ui, d, &format!("{base}.abovePpi"), " ppi", on);
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Compression:"));
        if key == "mono" {
            pick::<MonoCodec>(ui, d, &format!("{base}.compression"), 150.0, |_| true);
        } else {
            pick::<ImageCodec>(ui, d, &format!("{base}.compression"), 120.0, |_| true);
            let lossy = get(d, &format!("{base}.compression")).as_str().is_some_and(|c| c != ImageCodec::None.id() && c != ImageCodec::Zip.id());
            ui.label(tl!("Quality:"));
            ui.add_enabled_ui(lossy, |ui| {
                pick::<JpegQuality>(ui, d, &format!("{base}.quality"), 110.0, |_| true);
            });
        }
    });
}

fn compression(ui: &mut egui::Ui, d: &mut Dialog) {
    image_rows(ui, d, "color", tl!("Color images"));
    image_rows(ui, d, "gray", tl!("Grayscale images"));
    image_rows(ui, d, "mono", tl!("Monochrome images"));
    ui.add_space(8.0);
    flag(ui, d, "compression.compressText", tl!("Compress text and line art"), true);
}

pub(super) fn marks_and_bleeds(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let unit = app.session.general_unit();
    const MARKS: [(&str, &str); 4] = [
        ("marks.trim", "Trim marks"),
        ("marks.registration", "Registration marks"),
        ("marks.colorBars", "Color bars"),
        ("marks.pageInfo", "Page information"),
    ];
    heading(ui, tl!("Marks"));
    let all = MARKS.iter().all(|(p, _)| get(d, p).as_bool() == Some(true));
    if widgets::check(ui, tl!("All printer's marks"), all, true) {
        for (p, _) in MARKS {
            set(d, p, json!(!all));
        }
    }
    egui::Grid::new("pdf-marks").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
        for pair in MARKS.chunks(2) {
            for (p, label) in pair {
                flag(ui, d, p, tl!(label), true);
            }
            ui.end_row();
        }
    });
    row(ui, tl!("Printer mark type:"), |ui| {
        pick::<MarkKind>(ui, d, "marks.kind", 130.0, |_| true);
    });
    row(ui, tl!("Trim mark weight:"), |ui| {
        let w = get(d, "marks.weight").as_f64().unwrap_or(0.25);
        let current =
            WEIGHTS.iter().zip(WEIGHT_LABELS).find(|(x, _)| (*x - w).abs() < 1e-9).map_or_else(|| format!("{w} pt"), |(_, l)| l.to_string());
        if let Some(x) = widgets::dropdown(ui, "marks.weight", &current, &WEIGHT_LABELS, 130.0).and_then(|i| WEIGHTS.get(i)) {
            set(d, "marks.weight", json!(x));
        }
    });
    row(ui, tl!("Offset:"), |ui| length(ui, d, "marks.offset", unit, true));
    heading(ui, tl!("Bleeds"));
    flag(ui, d, "bleed.useDocument", tl!("Use document bleed settings"), true);
    // With the document's bleed the fields show it (Document Setup's `[top, bottom, left, right]`).
    let document = (get(d, "bleed.useDocument").as_bool() == Some(true)).then(|| app.session.active().map_or([0.0; 4], |s| s.doc.setup.bleed));
    egui::Grid::new("pdf-bleed").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
        for pair in [
            [(0, "bleed.top", tl!("Top:")), (1, "bleed.bottom", tl!("Bottom:"))],
            [(2, "bleed.left", tl!("Left:")), (3, "bleed.right", tl!("Right:"))],
        ] {
            for (i, p, label) in pair {
                ui.label(label);
                match document {
                    Some(b) => {
                        ui.add_enabled_ui(false, |ui| widgets::num_field(ui, p, b.get(i).copied(), unit, 80.0));
                    }
                    None => length(ui, d, p, unit, true),
                }
            }
            ui.end_row();
        }
    });
}

/// A dropdown of names such as colour profiles or flattener presets (`names`, the first one
/// standing for "" at `path`), showing the name at `path` (one that isn't among them too). The
/// first `builtins` entries are ours (translated); the others (profiles, saved presets) are names.
pub(super) fn profile_pick(ui: &mut egui::Ui, d: &mut Dialog, path: &str, names: &[&str], builtins: usize, enabled: bool) {
    let blank = names.first().copied().unwrap_or_default();
    let current = get(d, path).as_str().filter(|s| !s.is_empty()).unwrap_or(blank).to_string();
    ui.add_enabled_ui(enabled, |ui| {
        if let Some(i) = super::mixed_dropdown(ui, path, &current, names, 300.0, |k| k < builtins) {
            set(d, path, json!(if i == 0 { "" } else { names.get(i).copied().unwrap_or_default() }));
        }
    });
}

fn output(ui: &mut egui::Ui, d: &mut Dialog) {
    let profiles = vectorcraft_color::cms::profiles();
    let names = |blank: &'static str| -> Vec<&str> { std::iter::once(blank).chain(profiles.iter().map(|p| p.name.as_str())).collect() };
    let standard = choice::<Standard>(d, "standard").unwrap_or_default();
    heading(ui, tl!("Color"));
    row(ui, tl!("Color conversion:"), |ui| {
        pick::<ColorConversion>(ui, d, "output.conversion", 300.0, |_| true);
    });
    // PDF/X-1a converts to CMYK even without a conversion.
    let converting = get(d, "output.conversion") != ColorConversion::None.id() || standard.cmyk_only();
    // The document's own profile (blank), the CMYK profile in effect, then every profile.
    let destinations: Vec<&str> = std::iter::once(DOCUMENT_PROFILE)
        .chain(std::iter::once(vectorcraft_pdf::DOCUMENT_CMYK))
        .chain(profiles.iter().map(|p| p.name.as_str()))
        .collect();
    row(ui, tl!("Destination:"), |ui| profile_pick(ui, d, "output.destination", &destinations, 2, converting));
    row(ui, tl!("Profile inclusion:"), |ui| {
        // A standard decides whether colours are tagged.
        ui.add_enabled_ui(standard == Standard::None, |ui| pick::<ProfileInclusion>(ui, d, "output.profiles", 300.0, |_| true));
    });
    if standard.cmyk_only() {
        note(ui, tl!("PDF/X-1a files are CMYK: every colour is converted to the destination (blank: the output intent's CMYK profile), untagged."));
    } else if standard != Standard::None {
        note(ui, tl!("Files of this standard tag their colours with ICC profiles."));
    }
    heading(ui, tl!("Output Intent"));
    // PDF/A files carry their own output intent; PDF/X files always have one.
    let own = standard != Standard::PdfA2b;
    let blank = if standard.is_pdfx() { PDFX_INTENT } else { NO_PROFILE };
    row(ui, tl!("Output intent profile:"), |ui| profile_pick(ui, d, "output.outputIntent", &names(blank), 1, own));
    for (p, label) in [
        ("output.outputCondition", tl!("Output condition:")),
        ("output.outputConditionId", tl!("Condition identifier:")),
        ("output.registry", tl!("Registry name:")),
    ] {
        row(ui, label, |ui| text(ui, d, p, own));
    }
    flag(ui, d, "output.trapped", tl!("Mark as trapped"), own);
    if !own {
        note(ui, tl!("PDF/A files carry their own output intent."));
    }
}

fn advanced(app: &VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    heading(ui, tl!("Fonts"));
    let outline = get(d, "advanced.outlineText").as_bool() == Some(true);
    row(ui, tl!("Subset fonts below:"), |ui| {
        number(ui, d, "advanced.fontSubsetPercent", "%", !outline);
        ui.label(tl!("of characters used"));
    });
    flag(ui, d, "advanced.outlineText", tl!("Convert text to outlines"), true);
    heading(ui, tl!("Overprint and Transparency Flattener"));
    row(ui, tl!("Overprint:"), |ui| {
        pick::<Overprint>(ui, d, "advanced.overprint", 130.0, |_| true);
    });
    // PDF 1.3 files (PDF/X-1a and PDF/X-3 ones too) have no transparency: it is flattened.
    let flat = choice::<Compatibility>(d, "compatibility") == Some(Compatibility::Pdf13)
        || choice::<Standard>(d, "standard").is_some_and(Standard::flattens);
    row(ui, tl!("Flattener preset:"), |ui| {
        let presets = app.session.flattener_presets();
        let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        // The built-in presets come first (translated); the saved ones are names.
        let builtins = names.len().saturating_sub(app.session.prefs.flattener_presets.len());
        let name = get(d, "flattenerPreset").as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(pdf::DEFAULT_FLATTENER);
        // A built-in preset named by id (`high`) shows as its name.
        let current = FlattenOptions::preset_label(&name.to_ascii_lowercase()).unwrap_or(name).to_string();
        ui.add_enabled_ui(flat, |ui| {
            if let Some(n) = super::mixed_dropdown(ui, "flattenerPreset", &current, &names, 200.0, |k| k < builtins).and_then(|i| names.get(i)) {
                set(d, "flattenerPreset", json!(n));
            }
        });
    });
    if !flat {
        note(ui, tl!("PDF 1.3 files have no transparency: the preset flattens it."));
    }
}

/// The password fields (never stored in presets).
const OPEN_PASSWORD: &str = "security.openPassword";
const PERMISSIONS_PASSWORD: &str = "security.permissionsPassword";

/// A checkbox turning a password on (UI-only `flag`) or off (clearing it); checked while the
/// password is set. Returns whether the password applies.
fn password_check(ui: &mut egui::Ui, d: &mut Dialog, path: &str, flag: &str, label: &str, enabled: bool) -> bool {
    let on = d.bool(flag) || get(d, path).as_str().is_some_and(|s| !s.is_empty());
    if widgets::check(ui, label, on, enabled) {
        d.fields.insert(flag.into(), json!(!on));
        if on {
            set(d, path, json!(""));
        }
    }
    on && enabled
}

fn password_field(ui: &mut egui::Ui, d: &mut Dialog, path: &str, enabled: bool) {
    let mut s = get(d, path).as_str().unwrap_or_default().to_string();
    if ui.add_enabled(enabled, egui::TextEdit::singleline(&mut s).password(true).align(egui::Align2::LEFT_CENTER).desired_width(200.0)).changed() {
        set(d, path, json!(s));
    }
}

fn security(ui: &mut egui::Ui, d: &mut Dialog) {
    let plain = choice::<Standard>(d, "standard").unwrap_or_default() == Standard::None;
    if !plain {
        note(ui, tl!("Files of a PDF standard can't be password-protected."));
    }
    heading(ui, tl!("Document open password"));
    let open = password_check(ui, d, OPEN_PASSWORD, "__requireOpenPassword", tl!("Require a password to open the document"), plain);
    row(ui, tl!("Open password:"), |ui| password_field(ui, d, OPEN_PASSWORD, open));
    heading(ui, tl!("Permissions"));
    let restrict = password_check(ui, d, PERMISSIONS_PASSWORD, "__restrictPermissions", tl!("Restrict printing, editing and other tasks"), plain);
    row(ui, tl!("Permissions password:"), |ui| password_field(ui, d, PERMISSIONS_PASSWORD, restrict));
    ui.add_enabled_ui(restrict, |ui| {
        row(ui, tl!("Printing allowed:"), |ui| {
            pick::<Printing>(ui, d, "security.printing", 260.0, |_| true);
        });
        row(ui, tl!("Changes allowed:"), |ui| {
            pick::<Changes>(ui, d, "security.changes", 260.0, |_| true);
        });
    });
    flag(ui, d, "security.copy", tl!("Enable copying of text, images and other content"), restrict);
    let reader = tl!("Enable text access for screen readers");
    if get(d, "security.copy").as_bool() == Some(true) {
        // What can be copied can be read out.
        widgets::check(ui, reader, true, false);
    } else {
        flag(ui, d, "security.screenReader", reader, restrict);
    }
    let compatibility = choice::<Compatibility>(d, "compatibility").unwrap_or_default();
    flag(
        ui,
        d,
        "security.plaintextMetadata",
        tl!("Enable plaintext metadata"),
        (open || restrict) && !matches!(compatibility, Compatibility::Pdf13 | Compatibility::Pdf14),
    );
    ui.add_space(6.0);
    note(ui, &crate::i18n::fmt(tl!("Encryption: {name} (set by Compatibility)"), &[("name", Encryption::for_compatibility(compatibility).label())]));
}

/// The section a changed option belongs to (for the Summary's order).
pub(super) fn section_of(option: &str) -> usize {
    match option.split('.').next().unwrap_or_default() {
        "compression" => 1,
        "marks" | "bleed" => 2,
        "output" => 3,
        "advanced" => 4,
        "security" => 5,
        _ => 0,
    }
}

/// `compression.color.abovePpi` → "Compression › Color › Above Ppi": the section, then the keys
/// (without the section's own object), `thumbnails` → "General › Thumbnails".
pub(super) fn option_label(option: &str) -> String {
    let section = SECTIONS.get(section_of(option)).copied().unwrap_or(SECTIONS[0]);
    let mut keys = option.split('.').peekable();
    keys.next_if(|k| k.eq_ignore_ascii_case(section));
    std::iter::once(section.to_string()).chain(keys.map(|k| form::humanize(k).trim_end_matches(':').to_string())).collect::<Vec<_>>().join(" › ")
}

/// `document.pdfSettings` for the dialog with the document's warnings (an export in memory),
/// cached until a field or the document changes.
fn summary_of(app: &mut VectorcraftApp, ctx: &egui::Context, p: &Value) -> Result<Value, String> {
    let revision = app.session.active().map_or(0, |s| s.revision);
    let stamp = egui::Id::unique((p.to_string(), revision));
    let key = egui::Id::unique("save-pdf-summary");
    if let Some((s, v)) = ctx.data(|m| m.get_temp::<(egui::Id, Result<Value, String>)>(key))
        && s == stamp
    {
        return v;
    }
    let mut q = p.clone();
    q["includeDocument"] = json!(true);
    let v = app.session.execute("document.pdfSettings", &q).map_err(|e| e.to_string());
    ctx.data_mut(|m| m.insert_temp(key, (stamp, v.clone())));
    v
}

fn summary(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) {
    let t = Tokens::get(ui.ctx());
    let v = match summary_of(app, ui.ctx(), &params(d)) {
        Ok(v) => v,
        Err(e) => {
            heading(ui, tl!("Error"));
            ui.label(egui::RichText::new(format!("⚠ {e}")).color(t.text));
            return;
        }
    };
    heading(ui, tl!("Options"));
    let mut changed: Vec<&Value> = v["changed"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    changed.sort_by_key(|c| section_of(c["option"].as_str().unwrap_or_default()));
    if changed.is_empty() {
        let base = if d.kind == PRESET_KIND { pdf::DEFAULT_PRESET } else { tl!("the preset") };
        note(ui, &crate::i18n::fmt(tl!("Every option matches {base}."), &[("base", base)]));
    }
    option_rows(ui, changed, option_label);
    heading(ui, tl!("Warnings"));
    warning_rows(ui, v["warnings"].as_array().map(Vec::as_slice).unwrap_or_default());
}

/// A Summary line per changed option (`{option, value}`), labelled by `label`.
pub(super) fn option_rows<'a>(ui: &mut egui::Ui, changed: impl IntoIterator<Item = &'a Value>, label: impl Fn(&str) -> String) {
    let t = Tokens::get(ui.ctx());
    for c in changed {
        let value = match &c["value"] {
            Value::Bool(b) => if *b { tl!("On") } else { tl!("Off") }.to_string(),
            Value::String(s) if c["option"] == "marks.kind" => choice_label(crate::i18n::current(), "marks.kind", s).to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        ui.label(egui::RichText::new(format!("{}: {value}", label(c["option"].as_str().unwrap_or_default()))).color(t.text));
    }
}

/// The Summary's warnings ("None." without any).
pub(super) fn warning_rows(ui: &mut egui::Ui, warnings: &[Value]) {
    let t = Tokens::get(ui.ctx());
    if warnings.is_empty() {
        note(ui, tl!("None."));
    }
    for w in warnings {
        ui.label(egui::RichText::new(format!("⚠ {}", w.as_str().unwrap_or_default())).color(t.text));
    }
}

#[cfg(test)]
mod tests;
