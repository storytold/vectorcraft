//! EPS Options (Export As → EPS): the preview (none, TIFF black and white or colour, transparent
//! or opaque), overprints and the flattener preset transparency is flattened with, fonts, the
//! options (linked files, thumbnails, CMYK PostScript, compatible gradients) and the PostScript
//! language level.
//!
//! Fields: `document.export` params (`format: eps`, `path?`, the artboard choice) and the EPS
//! options of `document.exportEps`. OK remembers the options for next time and writes the file
//! (asking where when there is no path); bad options keep the dialog open.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::{FlattenOptions, fileio};
use vectorcraft_eps::{Level, Overprint, Preview};

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "epsOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("EPS Options").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 420.0,
    max_width: Some(460.0),
    inputs: |_| &["path"],
    ..DialogSpec::FORM
};

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 7] = ["format", "path", "useArtboards", "range", "artboard", "artboards", "selectedOnly"];

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
    fields.insert("format".into(), json!("eps"));
    // A built-in preset named by id (`medium`) shows as its name.
    if let Some(label) = fields.get("flattenerPreset").and_then(Value::as_str).and_then(|p| FlattenOptions::preset_label(&p.to_ascii_lowercase())) {
        fields.insert("flattenerPreset".into(), json!(label));
    }
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
    let names: Vec<(&str, &str)> = presets.iter().map(|p| (p.name.as_str(), p.name.as_str())).collect();
    choice(ui, d, "flattenerPreset", tl!("Preset:"), &names);
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
    io::export(app, Some("eps"), path, &params)
}
