//! EPS Options (Export As → EPS): the preview (none, TIFF black and white or colour, transparent
//! or opaque), overprints and the flattener preset transparency is flattened with, fonts, the
//! options (linked files, thumbnails, CMYK PostScript, compatible gradients) and the PostScript
//! language level.
//!
//! Fields: `document.export` params (`format: eps`, `path?`, the artboard choice) and the EPS
//! options of `document.exportEps`. OK remembers the options for next time and writes the file
//! (asking where when there is no path); bad options keep the dialog open.
//!
//! Custom… beside the flattener preset opens Custom Transparency Flattener Options over the
//! dialog: its OK sets every flattener option as `flattener` (the preset then shows [Custom]) and
//! goes back to EPS Options; Cancel goes back with the options as they were. Choosing a preset
//! drops the custom options.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::{FlattenOptions, FlattenerPreset, fileio};
use vectorcraft_eps::{Level, Overprint, Preview};

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "epsOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("EPS Options").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 420.0,
    max_width: Some(460.0),
    inputs: |_| &["path", "flattener"],
    ..DialogSpec::FORM
};

/// Custom Transparency Flattener Options, opened over EPS Options by Custom….
pub(super) const CUSTOM_KIND: &str = "epsFlattenerOptions";

pub(super) const CUSTOM_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Custom Transparency Flattener Options").into(),
    body: custom_body,
    confirm: custom_confirm,
    min_width: 420.0,
    ..DialogSpec::FORM
};

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 8] = ["format", "path", "useArtboards", "range", "artboard", "artboards", "selectedOnly", super::export_as::SELECTION_FIELD];

/// The options a first EPS Options dialog starts from (the export's defaults).
fn defaults() -> Map<String, Value> {
    let medium = FlattenOptions::preset_label(FlattenOptions::PRESETS[1]).unwrap_or_default();
    let mut m = Map::new();
    for (k, v) in [
        ("previewFormat", json!(Preview::default().id())),
        ("transparentPreview", json!(true)),
        ("overprints", json!(Overprint::default().id())),
        ("flattenerPreset", json!(medium)),
        ("includeLinkedFiles", json!(false)),
        ("thumbnails", json!(true)),
        ("cmykPostScript", json!(true)),
        ("compatibleGradients", json!(false)),
        ("level", json!(Level::default().id())),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// Open EPS Options for an export whose params (path, artboard choice, any EPS option) are
/// `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let mut fields = defaults();
    for given in [&app.ui.eps_options, params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    // A preset asked for without options of its own replaces the custom options used last.
    if params.get("flattenerPreset").is_some() && params.get("flattener").is_none() {
        fields.remove("flattener");
    }
    fields.insert("format".into(), json!("eps"));
    builtin_label(&mut fields);
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::subheader(ui, tl!("Preview"));
    let previews: Vec<(&str, &str)> = Preview::ALL.iter().map(|p| (p.id(), p.label())).collect();
    choice(ui, d, "previewFormat", tl!("Format:"), &previews);
    // Only the colour preview can keep transparency.
    let color = Preview::from_id(&d.str("previewFormat")) == Some(Preview::TiffColor);
    let transparent = d.bool("transparentPreview");
    ui.horizontal(|ui| {
        ui.add_space(LABEL + 8.0);
        for (label, on) in [(tl!("Transparent"), true), (tl!("Opaque"), false)] {
            if widgets::radio(ui, label, color && transparent == on, color) {
                d.fields.insert("transparentPreview".into(), json!(on));
            }
            ui.add_space(12.0);
        }
    });
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Transparency"));
    let overprints: Vec<(&str, &str)> = Overprint::ALL.iter().map(|o| (o.id(), o.label())).collect();
    choice(ui, d, "overprints", tl!("Overprints:"), &overprints);
    let presets = app.session.flattener_presets();
    let current = flattener(d, &app.session.prefs.flattener_presets).ok();
    let mut custom = false;
    widgets::label_row(ui, tl!("Preset:"), LABEL, |ui| {
        if let Some(p) = super::flatten::preset_dropdown(ui, "eps-flattener-preset", &presets, &d.str("flattenerPreset"), current.as_ref(), 180.0) {
            d.fields.insert("flattenerPreset".into(), json!(p.name));
            d.fields.remove("flattener");
        }
        custom = widgets::flat_button_fit(ui, tl!("Custom…"), 72.0).clicked();
    });
    if custom {
        *d = over(d, &app.session.prefs.flattener_presets);
        return false;
    }
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Fonts"));
    // Type is written as glyph outlines, which need no fonts.
    widgets::check(ui, tl!("Embed Fonts (for other applications)"), true, false);
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Options"));
    check(ui, d, "includeLinkedFiles", tl!("Include Linked Files"));
    check(ui, d, "thumbnails", tl!("Include Document Thumbnails"));
    check(ui, d, "cmykPostScript", tl!("Include CMYK PostScript in RGB Files"));
    check(ui, d, "compatibleGradients", tl!("Compatible Gradient and Gradient Mesh Printing"));
    ui.add_space(4.0);
    let levels: Vec<(&str, &str)> = Level::ALL.iter().map(|l| (l.id(), l.label())).collect();
    // A level set as a number (`level: 2`) reads as its id.
    if let Some(Value::Number(n)) = d.fields.get("level") {
        let id = n.to_string();
        d.fields.insert("level".into(), json!(id));
    }
    choice(ui, d, "level", tl!("PostScript:"), &levels);
    ui.add_space(8.0);
    super::form::caption(
        ui,
        if Level::from_id(&d.str("level")) == Some(Level::Two) || d.bool("compatibleGradients") {
            tl!("Transparency is flattened with the preset. Type is written as outlines; gradients as bands of colour.")
        } else {
            tl!("Transparency is flattened with the preset. Type is written as outlines; gradients stay smooth.")
        },
    );
    false
}

/// `ui.dialog.set` changed `field` of EPS Options `d`: a preset set there drops the Custom…
/// options, as choosing it in the dropdown does, and a built-in one named by id shows as its name.
pub(super) fn field_set(d: &mut Dialog, field: &str) {
    if field == "flattenerPreset" {
        d.fields.remove("flattener");
        builtin_label(&mut d.fields);
    }
}

/// A built-in preset named by id (`medium`) in `fields` shows as its name.
fn builtin_label(fields: &mut Map<String, Value>) {
    if let Some(label) = fields.get("flattenerPreset").and_then(Value::as_str).and_then(|p| FlattenOptions::preset_label(&p.to_ascii_lowercase())) {
        fields.insert("flattenerPreset".into(), json!(label));
    }
}

/// The flattener options EPS Options `d` exports with: its preset, with the Custom… options over it.
fn flattener(d: &Dialog, saved: &[FlattenerPreset]) -> Result<FlattenOptions, String> {
    FlattenOptions::for_export(d.fields.get("flattenerPreset").and_then(Value::as_str), d.fields.get("flattener"), saved)
}

/// Custom Transparency Flattener Options over EPS Options `parent`, on the options it exports with
/// (the default preset's while those are not valid), without alpha: EPS has none.
pub(super) fn over(parent: &Dialog, saved: &[FlattenerPreset]) -> Dialog {
    let mut o = flattener(parent, saved).unwrap_or_default();
    o.preserve_alpha = false;
    let mut d = Dialog::new(CUSTOM_KIND, json!({}));
    super::flatten::put_options(&mut d.fields, &o);
    d.fields.insert(super::PARENT.into(), serde_json::to_value(parent).unwrap_or_default());
    d
}

/// The options Custom Transparency Flattener Options `d` holds.
fn custom_options(d: &Dialog) -> Result<FlattenOptions, String> {
    FlattenOptions::from_params_with(&Value::Object(d.fields.clone()), &[])
}

/// Whether the EPS Options under `d` keep overprints (Overprints: Preserve).
fn overprints_kept(d: &Dialog) -> bool {
    super::parent(d).is_none_or(|p| Overprint::from_id(&p.str("overprints")) != Some(Overprint::Discard))
}

/// The flattener options; the two Preserve switches show what the export does and are set by the
/// format (no alpha) and by EPS Options' Overprints.
fn custom_body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let current = custom_options(d);
    // Values refused stay as entered, so that changing another option keeps them. The Preserve
    // switches can't be set here, so the fields keep their own values of them.
    let (mut o, unread) = super::flatten::shown_options(&d.fields, &current);
    o.preserve_alpha = false;
    o.preserve_overprints &= overprints_kept(d);
    super::flatten::fields_editor(ui, "eps-flattener", &mut d.fields, &mut o, &unread, &["preserveAlpha", "preserveOverprints"]);
    if let Err(e) = &current {
        ui.label(egui::RichText::new(e).color(Tokens::get(ui.ctx()).text_dim).size(11.5));
    }
    ui.add_space(8.0);
    super::form::caption(ui, tl!("EPS has no alpha transparency, and Overprints sets whether overprints are kept."));
    false
}

/// OK: EPS Options again, exporting with these options (a built-in preset under them, so that
/// deleting a saved preset leaves them whole); bad options keep the dialog open.
fn custom_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut o = custom_options(d)?;
    o.preserve_alpha = false;
    let mut parent = super::parent(d).ok_or("the dialog it was opened over is no longer open")?;
    if FlattenOptions::preset(&parent.str("flattenerPreset")).is_none() {
        let medium = FlattenOptions::preset_label(FlattenOptions::PRESETS[1]).unwrap_or_default();
        parent.fields.insert("flattenerPreset".into(), json!(medium));
    }
    parent.fields.insert("flattener".into(), serde_json::to_value(&o).map_err(|e| e.to_string())?);
    app.ui.dialog = Some(parent);
    Ok(Value::Null)
}

/// Check the options, remember them, and write the file(s).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = fileio::eps::resolve(&super::form::params(d), &app.session.prefs.flattener_presets)?;
    // Nothing selected keeps the dialog open too.
    if let Some(st) = app.session.active() {
        fileio::export_source(st, &params).map_err(|e| e.to_string())?;
    }
    app.ui.eps_options =
        Value::Object(d.fields.iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect());
    app.ui.dialog = None;
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    io::export(app, Some("eps"), path, &super::export_as::exported(d, params))
}
