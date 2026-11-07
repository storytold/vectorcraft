//! DXF Options (Export As → DXF): version, scale (1 unit of the art = N drawing units) and
//! lineweights, number of colours, the format of placed images, Preserve Appearance or Maximize
//! Editability, and Export Selected Art Only, Alter Paths for Appearance and Outline Text.
//!
//! Fields: `document.export` params (`format: dxf`, `path?`, the artboard choice) and the DXF
//! options of `document.exportDxf`. OK remembers the options for next time and writes the file
//! (asking where when there is no path); bad options keep the dialog open.

use serde_json::{Map, Value, json};
use vectorcraft_cad::{ColorDepth, DxfVersion, Preserve, RasterFormat};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::fileio;

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use crate::state::Dialog;
use crate::{VectorcraftApp, io, widgets};

pub(super) const KIND: &str = "dxfOptions";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("DXF Options").into(),
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

/// The options a first DXF Options dialog starts from (the export's defaults).
fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [
        ("version", json!(DxfVersion::default().id())),
        ("unit", json!(Unit::Millimeters.label())),
        ("scale", json!(1)),
        ("scaleLineweights", json!(false)),
        ("colors", json!(ColorDepth::default().id())),
        ("rasterFormat", json!(RasterFormat::default().id())),
        ("preserve", json!(Preserve::default().id())),
        ("alterPaths", json!(false)),
        ("outlineText", json!(false)),
        ("selectedOnly", json!(false)),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// Open DXF Options for an export whose params (path, artboard choice, any DXF option) are
/// `params`, over the options used last.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let mut fields = defaults();
    for given in [&app.ui.dxf_options, params] {
        if let Some(o) = given.as_object() {
            fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    fields.insert("format".into(), json!("dxf"));
    app.ui.dialog = Some(Dialog::new(KIND, Value::Object(fields)));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let versions: Vec<(&str, &str)> = DxfVersion::ALL.iter().map(|v| (v.id(), v.label())).collect();
    choice(ui, d, "version", tl!("Version:"), &versions);
    ui.add_space(4.0);
    scale_row(ui, d);
    ui.horizontal(|ui| {
        ui.add_space(LABEL + 8.0);
        check(ui, d, "scaleLineweights", tl!("Scale Lineweights"));
    });
    ui.add_space(4.0);
    let depths: Vec<(&str, &str)> = ColorDepth::ALL.iter().map(|c| (c.id(), c.label())).collect();
    choice(ui, d, "colors", tl!("Number of Colors:"), &depths);
    let version = DxfVersion::from_id(&d.str("version")).unwrap_or_default();
    if ColorDepth::from_id(&d.str("colors")) == Some(ColorDepth::True) && version < DxfVersion::R2004 {
        note(ui, tl!("True colour needs version 2004 or later: the nearest of 256 colours is written."));
    }
    let rasters: Vec<(&str, &str)> = RasterFormat::ALL.iter().map(|r| (r.id(), r.label())).collect();
    choice(ui, d, "rasterFormat", tl!("Raster File Format:"), &rasters);
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Export Option"));
    let preserve = Preserve::from_id(&d.str("preserve")).unwrap_or_default();
    ui.horizontal(|ui| {
        for p in Preserve::ALL {
            if widgets::radio(ui, p.label(), p == preserve, true) {
                d.fields.insert("preserve".into(), json!(p.id()));
            }
            ui.add_space(12.0);
        }
    });
    ui.add_space(4.0);
    let selection = app.session.active().is_some_and(|st| !st.selection.is_empty());
    if widgets::check(ui, tl!("Export Selected Art Only"), d.bool("selectedOnly") && selection, selection) {
        d.fields.insert("selectedOnly".into(), json!(!d.bool("selectedOnly")));
    }
    check(ui, d, "alterPaths", tl!("Alter Paths for Appearance"));
    // Preserve Appearance always outlines type.
    let appearance = preserve == Preserve::Appearance;
    if widgets::check(ui, tl!("Outline Text"), appearance || d.bool("outlineText"), !appearance) {
        d.fields.insert("outlineText".into(), json!(!d.bool("outlineText")));
    }
    ui.add_space(8.0);
    super::form::caption(
        ui,
        match preserve {
            Preserve::Appearance => tl!("Type becomes outlines, and strokes a CAD line can't draw become filled shapes."),
            Preserve::Editability => tl!("Type stays text, and every stroke is a line with its lineweight and dashes."),
        },
    );
    false
}

/// Scale: 1 [unit] = [scale] Units (the `unit` and `scale` fields; DXF import reads them too).
pub(super) fn scale_row(ui: &mut egui::Ui, d: &mut Dialog) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let units: Vec<&str> = Unit::ALL.iter().filter(|u| **u != Unit::FeetInches).map(|u| u.label()).collect();
    let unit = Unit::named(&d.str("unit")).unwrap_or(Unit::Millimeters);
    widgets::label_row(ui, tl!("Scale:"), LABEL, |ui| {
        ui.label(egui::RichText::new("1").color(t.text));
        if let Some(u) = widgets::dropdown(ui, "dxf-unit", unit.label(), &units, 120.0).and_then(|i| units.get(i)) {
            d.fields.insert("unit".into(), json!(u));
        }
        ui.label(egui::RichText::new("=").color(t.text));
        let scale = d.f64("scale", 1.0);
        if let Some(v) = widgets::plain_field(ui, "dxf-scale", scale, "", 3, 64.0) {
            let (lo, hi) = (*vectorcraft_cad::SCALE_RANGE.start(), *vectorcraft_cad::SCALE_RANGE.end());
            d.fields.insert("scale".into(), json!(v.clamp(lo, hi)));
        }
        ui.label(egui::RichText::new(tl!("Units")).color(t.text));
    });
}

/// A dim line under a field.
fn note(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(LABEL + 8.0);
        super::form::caption(ui, text);
    });
}

/// Check the options, remember them, and write the file(s).
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = super::form::params(d);
    fileio::dxf::check(&params)?;
    // Nothing selected keeps the dialog open too.
    if let Some(st) = app.session.active() {
        fileio::export_source(st, &params).map_err(|e| e.to_string())?;
    }
    app.ui.dxf_options =
        Value::Object(d.fields.iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect());
    app.ui.dialog = None;
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    io::export(app, Some("dxf"), path, &params)
}
