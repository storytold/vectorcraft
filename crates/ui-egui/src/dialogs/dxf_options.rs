//! DXF Options (Export As → DXF): a preset (Custom, or CNC / CAM, which sets the options below for
//! cutting software and shows Custom again once one is changed), version, scale (1 unit of the
//! art = N drawing units) and lineweights, number of colours, the format of placed images and
//! Include Placed Images, Preserve Appearance or Maximize Editability, and Export Selected Art
//! Only, Alter Paths for Appearance, Outline Text and Include Fill Hatches.
//!
//! Fields: `document.export` params (`format: dxf`, `path?`, the artboard choice) and the DXF
//! options of `document.exportDxf`. OK remembers the options for next time and writes the file
//! (asking where when there is no path); bad options keep the dialog open.

use serde_json::{Map, Value, json};
use vectorcraft_cad::{ColorDepth, DxfPreset, DxfVersion, Preserve, RasterFormat};
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

/// The preset field (the `dxfPreset` param).
const PRESET: &str = "dxfPreset";
/// The preset the fields were last set from or matched (a change of [`PRESET`] sets its fields).
const SHOWN_PRESET: &str = "__shownPreset";

/// The options a first DXF Options dialog starts from (the export's defaults).
fn defaults() -> Map<String, Value> {
    let mut m = preset_fields(DxfPreset::Custom);
    m.insert(PRESET.into(), json!(DxfPreset::Custom.id()));
    m.insert("selectedOnly".into(), json!(false));
    m
}

/// The fields `preset` sets (Custom: the export's defaults).
fn preset_fields(preset: DxfPreset) -> Map<String, Value> {
    let o = preset.options();
    let mut m = Map::new();
    for (k, v) in [
        ("version", json!(o.version.id())),
        ("unit", json!(o.unit.label())),
        ("scale", json!(o.scale)),
        ("scaleLineweights", json!(o.scale_lineweights)),
        ("colors", json!(o.colors.id())),
        ("rasterFormat", json!(o.raster.id())),
        ("includeImages", json!(o.include_images)),
        ("includeHatches", json!(o.include_hatches)),
        ("preserve", json!(o.preserve.id())),
        ("alterPaths", json!(o.alter_paths)),
        ("outlineText", json!(o.outline_text)),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// Show the preset the fields match: CNC / CAM while every field it sets is as it sets it, else
/// Custom. An unknown preset stays as it is, for OK to refuse.
fn show_matching_preset(d: &mut Dialog) {
    if DxfPreset::from_id(&d.str(PRESET)).is_none() {
        return;
    }
    let preset = fileio::dxf::options(&super::form::params(d)).map_or(DxfPreset::Custom, |o| DxfPreset::of(&o));
    d.fields.insert(PRESET.into(), json!(preset.id()));
    d.fields.insert(SHOWN_PRESET.into(), json!(preset.id()));
}

/// A preset picked since the fields were last matched, or any preset when `force` is set, sets
/// its fields; Custom puts back the export's defaults.
fn apply_picked_preset(d: &mut Dialog, force: bool) {
    if (force || !d.str(PRESET).eq_ignore_ascii_case(&d.str(SHOWN_PRESET)))
        && let Some(p) = DxfPreset::from_id(&d.str(PRESET))
    {
        d.fields.extend(preset_fields(p));
    }
}

/// `field` was set (`ui.dialog.set`): a preset sets its fields now, even the one shown, so fields
/// set after it keep their values, and any other field shows the preset the fields then match, as
/// drawing does.
pub(super) fn note_set(d: &mut Dialog, field: &str) {
    if field == PRESET {
        apply_picked_preset(d, true);
    }
    show_matching_preset(d);
}

/// Open DXF Options for an export whose params (path, artboard choice, any DXF option) are
/// `params`, over the options used last. A `dxfPreset` in `params` sets its fields under the
/// options `params` gives.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    let mut fields = defaults();
    if let Some(o) = app.ui.dxf_options.as_object() {
        fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if let Some(o) = params.as_object() {
        if let Some(p) = o.get(PRESET).and_then(Value::as_str).and_then(DxfPreset::from_id) {
            fields.extend(preset_fields(p));
        }
        fields.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    fields.insert("format".into(), json!("dxf"));
    let mut d = Dialog::new(KIND, Value::Object(fields));
    show_matching_preset(&mut d);
    app.ui.dialog = Some(d);
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let presets: Vec<(&str, &str)> = DxfPreset::ALL.iter().map(|p| (p.id(), p.label())).collect();
    choice(ui, d, PRESET, tl!("Preset:"), &presets);
    apply_picked_preset(d, false);
    ui.add_space(4.0);
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
    ui.horizontal(|ui| {
        ui.add_space(LABEL + 8.0);
        check(ui, d, "includeImages", tl!("Include Placed Images"));
    });
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
    check(ui, d, "includeHatches", tl!("Include Fill Hatches"));
    ui.add_space(8.0);
    super::form::caption(
        ui,
        match preserve {
            Preserve::Appearance => tl!("Type becomes outlines, and strokes a CAD line can't draw become filled shapes."),
            Preserve::Editability => tl!("Type stays text, and every stroke is a line with its lineweight and dashes."),
        },
    );
    show_matching_preset(d);
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
    // A preset or field set since the dialog last drew (the control channel sets fields without
    // drawing) counts as drawn.
    let mut d = d.clone();
    apply_picked_preset(&mut d, false);
    show_matching_preset(&mut d);
    let d = &d;
    let params = super::form::params(d);
    fileio::dxf::check(&params)?;
    // Nothing selected keeps the dialog open too.
    if let Some(st) = app.session.active() {
        fileio::export_source(st, &params).map_err(|e| e.to_string())?;
    }
    let remembered = params.as_object().map(|o| o.iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())));
    app.ui.dxf_options = Value::Object(remembered.into_iter().flatten().collect());
    app.ui.dialog = None;
    let path = d.fields.get("path").and_then(Value::as_str).map(str::to_string);
    io::export(app, Some("dxf"), path, &params)
}
