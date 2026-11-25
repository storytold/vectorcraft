//! DXF Options (Export As → DXF): a preset (Custom, or CNC / CAM, which sets the options below for
//! cutting software and shows Custom again once one is changed), version, scale (1 unit of the
//! art = N drawing units) and lineweights, number of colours, the format of placed images and
//! Include Placed Images, Preserve Appearance or Maximize Editability, and Export Selected Art
//! Only, Alter Paths for Appearance, Outline Text and Include Fill Hatches.
//!
//! Fields: `document.export` params (`format: dxf`, `path?`, the artboard choice) and the DXF
//! options of `document.exportDxf`. OK remembers the options for next time and writes the file
//! (asking where when there is no path); bad options keep the dialog open.

use craft::ui::contract::{Bind, Entry, Show as _};
use craft::ui::display::{Shade, Text};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::style::TypeRole;
use craft::ui::text::NumberField;
use serde_json::{Map, Value, json};
use vectorcraft_cad::{ColorDepth, DxfPreset, DxfVersion, Preserve, RasterFormat};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::fileio;

use super::export_host::{self, After, Export, Open};
use super::form;
use crate::{VectorcraftApp, io};

pub(super) const KIND: &str = "dxfOptions";

/// Fields that are not remembered between exports (they belong to one export).
const PER_EXPORT: [&str; 7] = ["format", "path", "useArtboards", "range", "artboard", "artboards", "selectedOnly"];

/// The preset field (the `dxfPreset` param).
const PRESET: &str = "dxfPreset";

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

/// `map` as the dialog's values.
fn dialog_fields(map: &Map<String, Value>) -> DialogFields {
    form::fields_of(&kinds(), map)
}

/// The rows as far as their kinds go, for reading values to and from JSON.
fn kinds() -> Vec<Param> {
    rows(false, false, false)
}

/// Gives the dialog the fields the current preset sets and the others have lost (a change of the preset row removes
/// them), then shows the preset the fields match: CNC / CAM while every field it sets is as it sets it, else Custom.
/// An unknown preset stays as it is, for OK to refuse.
fn settle(fields: &mut DialogFields) {
    let Some(preset) = fields.text(PRESET).and_then(DxfPreset::from_id) else { return };
    for (key, value) in dialog_fields(&preset_fields(preset)).iter() {
        if !fields.contains(key) {
            fields.set(key, value.clone());
        }
    }
    let shown = fileio::dxf::options(&Value::Object(form::json_of(&kinds(), fields))).map_or(DxfPreset::Custom, |o| DxfPreset::of(&o));
    fields.set(PRESET, shown.id());
}

/// A choice row over `rows` (id, label).
pub(super) fn choice_param(key: &str, label: &'static str, rows: impl IntoIterator<Item = (&'static str, &'static str)>) -> Param {
    let options = rows.into_iter().map(|(id, name)| (id.to_string(), tl!(name).into())).collect();
    Param::new(key, label, ParamKind::choice(options, None))
}

/// A radio row over `rows` (id, label).
pub(super) fn radio_param(key: &str, label: &'static str, rows: impl IntoIterator<Item = (&'static str, &'static str)>) -> Param {
    let options = rows.into_iter().map(|(id, name)| (id.to_string(), tl!(name).into())).collect();
    Param::new(key, label, ParamKind::radio(options, None))
}

/// A check box row.
pub(super) fn check_param(key: &str, label: &'static str) -> Param {
    Param::new(key, label, ParamKind::bool(false))
}

/// The rows. Export Selection leaves Export Selected Art Only out, as it writes the selected art alone;
/// without a selection that row is off. `note` shows the true-colour note.
fn rows(selection: bool, has_selection: bool, note: bool) -> Vec<Param> {
    let keys: Vec<String> = preset_fields(DxfPreset::Custom).keys().cloned().collect();
    let mut rows = vec![
        choice_param(PRESET, tl!("Preset:"), DxfPreset::ALL.iter().map(|p| (p.id(), p.label()))).resets(keys),
        choice_param("version", tl!("Version:"), DxfVersion::ALL.iter().map(|v| (v.id(), v.label()))),
        scale_param(None),
        check_param("scaleLineweights", tl!("Scale Lineweights")),
        choice_param("colors", tl!("Number of Colors:"), ColorDepth::ALL.iter().map(|c| (c.id(), c.label()))),
    ];
    if note {
        rows.push(Param::read_only_text("__colorNote", "", tl!("True colour needs version 2004 or later: the nearest of 256 colours is written.")));
    }
    rows.extend([
        choice_param("rasterFormat", tl!("Raster File Format:"), RasterFormat::ALL.iter().map(|r| (r.id(), r.label()))),
        check_param("includeImages", tl!("Include Placed Images")),
        Param::section(tl!("Export Option")),
        radio_param("preserve", "", Preserve::ALL.iter().map(|p| (p.id(), p.label()))),
    ]);
    if !selection {
        let row = check_param("selectedOnly", tl!("Export Selected Art Only"));
        rows.push(if has_selection { row } else { row.enabled_when_in("selectedOnly", std::iter::empty::<FieldValue>()) });
    }
    rows.extend([
        check_param("alterPaths", tl!("Alter Paths for Appearance")),
        // Preserve Appearance always outlines type.
        check_param("outlineText", tl!("Outline Text")).enabled_when("preserve", FieldValue::Text(Preserve::Editability.id().into())),
        check_param("includeHatches", tl!("Include Fill Hatches")),
    ]);
    rows
}

/// Scale: 1 [unit] = [scale] Units (the `unit` and `scale` fields; DXF import reads them too). The row is disabled
/// while the check box `disabled_when` names holds the value it gives.
pub(crate) fn scale_param(disabled_when: Option<(&str, FieldValue)>) -> Param {
    let row = Param::custom("scale", tl!("Scale:"), |ui, fields, _| {
        let units: Vec<Entry<String>> =
            Unit::ALL.iter().filter(|u| **u != Unit::FeetInches).map(|u| Entry::new(u.label().to_string(), u.label())).collect();
        let mut unit = fields.text("unit").map_or_else(|| Unit::Millimeters.label().to_string(), str::to_string);
        let mut scale = Some(fields.number("scale").unwrap_or(1.0));
        let (lo, hi) = (*vectorcraft_cad::SCALE_RANGE.start(), *vectorcraft_cad::SCALE_RANGE.end());
        let shown = ui
            .horizontal(|ui| {
                Text::new("1").show(ui);
                Select::new(tl!("Unit"), Bind::bound(&mut unit), &units).show(ui);
                Text::new("=").show(ui);
                NumberField::new(tl!("Scale"), Bind::bound(&mut scale)).range(lo..=hi).show(ui);
                Text::new(tl!("Units")).show(ui);
            })
            .response;
        let scale = scale.unwrap_or(1.0);
        if fields.text("unit") != Some(unit.as_str()) {
            fields.set("unit", unit);
        }
        if fields.number("scale") != Some(scale) {
            fields.set("scale", scale);
        }
        shown
    })
    .companions(["unit"]);
    match disabled_when {
        Some((key, FieldValue::Bool(v))) => row.enabled_when(key, FieldValue::Bool(!v)),
        _ => row,
    }
}

/// Open DXF Options for an export whose params (path, artboard choice, any DXF option) are
/// `params`, over the options used last. A `dxfPreset` in `params` sets its fields under the
/// options `params` gives.
pub fn open(app: &mut VectorcraftApp, params: &Value) {
    open_with(app, params, false);
}

/// [`open`] for Export Selection when `selection` is set: the options then write the selected art alone.
pub fn open_with(app: &mut VectorcraftApp, params: &Value, selection: bool) {
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
    let mut values = dialog_fields(&fields);
    settle(&mut values);
    let has_selection = app.session.active().is_some_and(|st| !st.selection.is_empty());
    let ctx = app.ui_ctx.clone().unwrap_or_default();
    let dialog = HostedParamDialog::new(&ctx, KIND, values, move |fields| {
        settle(fields);
        let note = fields.text("colors").and_then(ColorDepth::from_id) == Some(ColorDepth::True)
            && fields.text("version").and_then(DxfVersion::from_id).unwrap_or_default() < DxfVersion::R2004;
        ParamDialog::new(KIND, tl!("DXF Options"), rows(selection, has_selection, note))
            .values(Bind::bound(fields))
            .ok_label(tl!("Export").into())
            .below(|ui, fields| {
                Text::new(match fields.text("preserve").and_then(Preserve::from_id).unwrap_or_default() {
                    Preserve::Appearance => tl!("Type becomes outlines, and strokes a CAD line can't draw become filled shapes."),
                    Preserve::Editability => tl!("Type stays text, and every stroke is a line with its lineweight and dashes."),
                })
                .role(TypeRole::Caption)
                .shade(Shade::Muted)
                .show(ui);
            })
    });
    export_host::open(app, Export::Dxf, selection, dialog);
}

/// Check the options, remember them, and write the file(s).
pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    // A preset or field set since the dialog last drew (the control channel sets fields without
    // drawing) counts as drawn.
    let mut fields = fields.clone();
    settle(&mut fields);
    let map = form::json_of(&rows(open.selection, true, false), &fields);
    let params = Value::Object(map.clone());
    fileio::dxf::check(&params)?;
    // Nothing selected keeps the dialog open too.
    if let Some(st) = app.session.active() {
        fileio::export_source(st, &params).map_err(|e| e.to_string())?;
    }
    app.ui.dxf_options = Value::Object(map.into_iter().filter(|(k, _)| !PER_EXPORT.contains(&k.as_str())).collect());
    let path = fields.text("path").map(str::to_string);
    io::export(app, Some("dxf"), path, &super::export_as::exported(open.selection, params)).map(|_| None)
}
