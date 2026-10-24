//! CorelDRAW Export Options (File › Export › Export As CDR, Export As → CorelDRAW): the target
//! version (the ones verified in CorelDRAW and the experimental ones), editable type or outlines,
//! vector paths always kept, permission to rasterize raster effects, and post-export validation.
//!
//! Fields: `document.export` params (`format: cdr`, `path?`) and the CDR options: `version` (the
//! number as text), `textToOutlines`, `validate`, `rasterizeEffects`. OK remembers the options and
//! writes the file (asking where when there is no path); bad options keep the dialog open.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use super::document_setup::{check, choice};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "cdrOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("CorelDRAW Export Options").into(),
    body,
    confirm,
    ok: Some("Export"),
    min_width: 440.0,
    max_width: Some(500.0),
    ..DialogSpec::FORM
};

/// Fields that belong to one export only.
const PER_EXPORT: [&str; 2] = ["format", "path"];

fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("version".into(), json!(vectorcraft_cdr::DEFAULT_EXPORT_VERSION.to_string()));
    m.insert("textToOutlines".into(), json!(false));
    m.insert("validate".into(), json!(true));
    m.insert("rasterizeEffects".into(), json!(false));
    m
}

/// Open the dialog for an export with params `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let mut fields = defaults();
    for given in [&app.ui.cdr_options, params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    if let Some(v) = fields.get("version").and_then(Value::as_u64) {
        fields.insert("version".into(), json!(v.to_string()));
    }
    fields.insert("format".into(), json!("cdr"));
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

/// File › Export › Export As CDR…: ask where to write, then open the options.
pub fn export_command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if app.session.active().is_none() {
        return Err("no document".into());
    }
    let given = p.get("path").and_then(Value::as_str).map(str::to_string);
    let path = io::target_path(app, given, "cdr")?;
    let mut params = p.clone();
    if !params.is_object() {
        params = json!({});
    }
    params["path"] = json!(path);
    open(app, &params);
    Ok(Value::Null)
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let versions: Vec<(String, String)> = vectorcraft_cdr::export_versions()
        .into_iter()
        .rev()
        .map(|v| {
            let tag = match v.status {
                vectorcraft_cdr::ExportStatus::Verified => tl!("verified"),
                vectorcraft_cdr::ExportStatus::Experimental => tl!("experimental"),
            };
            (v.version.to_string(), format!("{} — {} ({tag})", v.version, v.product))
        })
        .collect();
    let rows: Vec<(&str, &str)> = versions.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    choice(ui, d, "version", tl!("Target Version:"), &rows);
    let version: u16 = d.str("version").parse().unwrap_or(vectorcraft_cdr::DEFAULT_EXPORT_VERSION);
    super::form::caption(
        ui,
        if version >= vectorcraft_cdr::VERIFIED_SINCE {
            tl!(
                "Files of this version were verified to open in CorelDRAW. Older CorelDRAW releases can't open newer versions: pick the oldest one you need."
            )
        } else {
            tl!("Experimental: written with the layout of that version's sample files, but not verified to open in CorelDRAW.")
        },
    );
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Text"));
    let outlines = d.bool("textToOutlines");
    if widgets::radio(ui, tl!("Keep Text Editable"), !outlines, true) {
        d.fields.insert("textToOutlines".into(), json!(false));
    }
    if widgets::radio(ui, tl!("Convert Text to Outlines"), outlines, true) {
        d.fields.insert("textToOutlines".into(), json!(true));
    }
    super::form::caption(ui, tl!("Area type, vertical type and type on a path are always written as outlines."));
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Compatibility"));
    // Vectors never become images: the box stays checked and disabled.
    widgets::check(ui, tl!("Preserve Vector Paths"), true, false);
    check(ui, d, "rasterizeEffects", tl!("Rasterize Raster Effects (Shadows, Glows, Blurs)"));
    check(ui, d, "validate", tl!("Validate After Export"));
    if !d.bool("validate") {
        super::form::caption(ui, tl!("Without validation the written file isn't checked."));
    }
    false
}

/// Check the options, remember them, and write the file.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = super::form::params(d);
    let version: u16 = d.str("version").parse().unwrap_or(vectorcraft_cdr::DEFAULT_EXPORT_VERSION);
    params["version"] = json!(version);
    fileio::cdr::check(&params)?;
    app.ui.cdr_options =
        Value::Object(d.fields.iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect());
    app.ui.dialog = None;
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    app.status(tl!("Exporting to CorelDRAW…").to_string());
    let r = io::export(app, Some("cdr"), path.clone(), &params);
    if r.is_ok() {
        let done = if d.bool("validate") { tl!("CorelDRAW file exported and validated") } else { tl!("CorelDRAW file exported (not validated)") };
        app.status(format!("{}: {}", done, path.as_deref().map(fileio::file_name).unwrap_or_default()));
    }
    r
}
