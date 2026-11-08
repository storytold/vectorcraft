//! Opening and placing DXF drawings: the `dxf` options `document.open` and `file.place` read
//! (layout, scale or fit, lineweights, centring, merged layers) and `document.dxfInfo` (layouts,
//! layers, units) for the DXF Import Options dialog.

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_cad::ImportOptions;
use vectorcraft_doc::{Document, Unit};

use super::super::{CommandSpec, Result, Session, always, bad, cmd};
use super::{err, source};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "document.dxfInfo",
        "DXF Info",
        [],
        None,
        "{path} or {name?, dataBase64} what a DXF drawing holds → {version (2018, R12…), units (its drawing unit: Millimeters, Inches, Unitless…), layouts: [\"Model\", paper layouts…], layers: [name…], unit, scale (the default ratio: 1 unit = scale drawing units, the drawing at 1:1)}; a binary DXF or a DWG answers an error saying to save it as ASCII DXF",
        always,
        dxf_info
    )]
}

/// The `dxf` params, as they come.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Params {
    layout: Option<String>,
    fit: Option<bool>,
    fit_to: Option<[f64; 2]>,
    unit: Option<String>,
    scale: Option<f64>,
    scale_lineweights: Option<bool>,
    center: Option<bool>,
    merge_layers: Option<bool>,
}

/// The DXF import options in `p` (the `dxf` object of `document.open` and `file.place`).
pub(super) fn options(cmd: &str, p: Option<&Value>) -> Result<ImportOptions> {
    let d = ImportOptions::default();
    let Some(p) = p.filter(|v| !v.is_null()) else { return Ok(d) };
    if !p.is_object() {
        return Err(bad(cmd, "dxf must be an object of DXF import options"));
    }
    let q = Params::deserialize(p).map_err(|e| bad(cmd, format!("dxf: {e}")))?;
    let unit = match q.unit.as_deref() {
        None => None,
        Some(u) => Some(Unit::named(u).ok_or_else(|| bad(cmd, format!("dxf unit `{u}`: a unit such as mm, cm, in or pt")))?),
    };
    if let Some(s) = q.scale
        && !(s.is_finite() && vectorcraft_cad::SCALE_RANGE.contains(&s))
    {
        return Err(bad(cmd, format!("dxf scale must be a positive number up to {}, not {s}", vectorcraft_cad::SCALE_RANGE.end())));
    }
    let fit_to = match q.fit_to {
        None => d.fit_to,
        Some([w, h]) if w > 0.0 && h > 0.0 && w.max(h) <= crate::MAX_COORD => (w, h),
        Some(_) => return Err(bad(cmd, "dxf fitTo must be [width, height], positive")),
    };
    Ok(ImportOptions {
        layout: q.layout.filter(|l| !l.trim().is_empty()),
        fit: q.fit.unwrap_or(d.fit),
        fit_to,
        unit,
        scale: q.scale,
        scale_lineweights: q.scale_lineweights.unwrap_or(d.scale_lineweights),
        center: q.center.unwrap_or(d.center),
        merge_layers: q.merge_layers.unwrap_or(d.merge_layers),
    })
}

/// Import a DXF drawing → the document and import notes.
pub(super) fn import(bytes: &[u8], o: &ImportOptions) -> Result<(Document, Vec<String>)> {
    let r = vectorcraft_cad::import(bytes, o).map_err(err)?;
    Ok((r.document, r.warnings))
}

const INFO: &str = "document.dxfInfo";

fn dxf_info(_: &mut Session, p: &Value) -> Result<Value> {
    let bytes = source(p, INFO)?.bytes;
    let i = vectorcraft_cad::info(&bytes).map_err(err)?;
    Ok(json!({
        "version": i.version,
        "units": i.units,
        "layouts": i.layouts,
        "layers": i.layers,
        "unit": i.unit.label(),
        "scale": i.scale,
    }))
}
