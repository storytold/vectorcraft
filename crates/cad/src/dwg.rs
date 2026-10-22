//! DWG R13–2018 import through cadkit's clean-room reader (MIT OR Apache-2.0).
//!
//! The decoded drawing is written to in-memory ASCII DXF, then passed through our existing
//! importer so CAD scale, layouts, layers, blocks and editable art have the same semantics.
//! Both decoding and conversion warnings reach the caller. No external converter is run.
//! Specification: <https://www.opendesign.com/files/guestdownloads/OpenDesign_Specification_for_.dwg_files.pdf> (DWG specification).

use cadkit_core::{Limits, ReadOptions};

use crate::{DxfInfo, DxfVersion, ImportOptions, Imported};

/// Recognize the DWG family, including older versions so they get an actionable error.
pub fn is_dwg(bytes: &[u8]) -> bool {
    bytes.starts_with(b"AC10") || bytes.starts_with(b"AC1.")
}

// Keep decompression and the intermediate CAD/DXF representations within desktop and WASM
// budgets. Raw object bytes aren't retained. These limits are checked by the reader before
// allocations/loops; the final DXF reader also enforces its own pair and expansion limits.
fn options() -> ReadOptions {
    ReadOptions {
        limits: Limits {
            max_input_bytes: 64 << 20,
            max_decompressed_bytes: 128 << 20,
            max_objects: 100_000,
            max_depth: 32,
            max_string_bytes: 64 << 10,
            max_vertices: 100_000,
            max_total_vertices: 500_000,
        },
        ..Default::default()
    }
}

fn decode(bytes: &[u8]) -> Result<cadkit_core::Document, String> {
    if !cadkit_dwg::sniff(bytes) {
        return Err("unsupported DWG version: open a DWG saved as R13–2018, or export ASCII DXF from the CAD app".into());
    }
    cadkit_dwg::read(bytes, &options()).map_err(|e| format!("DWG: {e}"))
}

fn as_dxf(doc: &cadkit_core::Document) -> Result<(String, Vec<String>), String> {
    let (mut text, losses) = cadkit_dxf::write_with_report(doc, cadkit_dxf::DxfVersion::R2018).map_err(|e| format!("DWG conversion: {e}"))?;
    // cadkit's DXF writer uses generic header defaults. Preserve the source's imperial/metric
    // fallback for unitless drawings and global dash scale, which our importer understands.
    let mut header = String::new();
    if let Some(cadkit_core::Value::Int(m @ (0 | 1))) = doc.props.get("dwg.measurement") {
        header.push_str(&format!("  9\r\n$MEASUREMENT\r\n 70\r\n{m}\r\n"));
    }
    if let Some(cadkit_core::Value::Float(scale)) = doc.props.get("dwg.header.LTSCALE")
        && scale.is_finite()
        && *scale > 0.0
    {
        header.push_str(&format!("  9\r\n$LTSCALE\r\n 40\r\n{scale}\r\n"));
    }
    if let Some(end) = text.find("  0\r\nENDSEC\r\n") {
        text.insert_str(end, &header);
    }
    let warnings = doc.warnings.iter().chain(&losses).map(|w| format!("DWG ({}): {}", w.code, w.message)).collect();
    Ok((text, warnings))
}

/// Inspect DWG units, layers and layouts for the CAD import dialog.
pub fn info(bytes: &[u8]) -> Result<DxfInfo, String> {
    let mut drawing = decode(bytes)?;
    // Inspection needs only tables and layout names, not another copy of all the geometry.
    for model in &mut drawing.models {
        model.entities.clear();
    }
    drawing.blocks.clear();
    let (text, _) = as_dxf(&drawing)?;
    let mut info = crate::info(text.as_bytes())?;
    // Report the source's version, not the bridge's DXF version.
    info.version = DxfVersion::from_id(&drawing.source.version).map_or(drawing.source.version, |v| v.id().into());
    Ok(info)
}

/// Import a DWG drawing as editable paths, text and symbols, with explicit loss warnings.
pub fn import(bytes: &[u8], opts: &ImportOptions) -> Result<Imported, String> {
    let drawing = decode(bytes)?;
    let (text, mut warnings) = as_dxf(&drawing)?;
    // Drop the decoded model before constructing the editable document.
    drop(drawing);
    let mut imported = crate::import(text.as_bytes(), opts)?;
    warnings.append(&mut imported.warnings);
    imported.warnings = warnings;
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{NodeKind, Unit};

    const OLD: &[u8] = include_bytes!("../tests/fixtures/dwg/drawing-2000.dwg");
    const MODERN: &[u8] = include_bytes!("../tests/fixtures/dwg/drawing-2018.dwg");

    #[test]
    fn dwg_versions_keep_editable_geometry_text_layers_and_units() {
        for (bytes, version) in [(OLD, "2000"), (MODERN, "2018")] {
            let info = info(bytes).unwrap();
            assert_eq!(info.version, version);
            assert_eq!(info.unit, Unit::Millimeters);
            assert!(info.layouts.contains(&"Model".into()));
            assert!(info.layers.contains(&"Geometry".into()));
            let imported = import(bytes, &ImportOptions { center: false, ..Default::default() }).unwrap();
            let mut paths = 0;
            let mut texts = Vec::new();
            imported.document.walk(|node| match &node.kind {
                NodeKind::Path { path, .. } => {
                    paths += 1;
                    let bounds = path.bounds().unwrap();
                    let width_mm = bounds.width() / Unit::Millimeters.points();
                    assert!((width_mm - 100.0).abs() < 1e-6 || (width_mm - 20.0).abs() < 1e-6, "{bounds:?}");
                }
                NodeKind::Text(t) => texts.push(t.plain_text()),
                _ => {}
            });
            assert_eq!(paths, 2, "{version}: {:?}", imported.warnings);
            assert_eq!(texts, ["VectorCraft CAD"], "{version}: {:?}", imported.warnings);
        }
    }

    #[test]
    fn dwg_limits_are_enforced_and_bad_versions_are_actionable() {
        let mut options = options();
        options.limits.max_input_bytes = 100;
        assert!(cadkit_dwg::read(OLD, &options).is_err());
        options = super::options();
        options.limits.max_objects = 1;
        assert!(cadkit_dwg::read(OLD, &options).is_err());
        options = super::options();
        options.limits.max_decompressed_bytes = 100;
        assert!(cadkit_dwg::read(MODERN, &options).is_err());
        let error = import(b"AC1009", &ImportOptions::default()).unwrap_err();
        assert!(error.contains("ASCII DXF"));
        for magic in [b"AC1012", b"AC1014", b"AC1015", b"AC1018", b"AC1021", b"AC1024", b"AC1027", b"AC1032"] {
            assert!(is_dwg(magic));
            assert!(import(magic, &ImportOptions::default()).is_err());
        }
    }

    #[test]
    fn dwg_bridge_preserves_reader_and_conversion_warnings() {
        let mut drawing = decode(OLD).unwrap();
        drawing.warnings.push(cadkit_core::Warning { code: "test.loss".into(), message: "unsupported data".into(), offset: None, object: None });
        drawing.models[0].entities.push(cadkit_core::Entity::new(cadkit_core::EntityKind::Unknown { type_name: "custom-fixture-entity".into() }));
        let (_, warnings) = as_dxf(&drawing).unwrap();
        assert!(warnings.iter().any(|w| w.contains("test.loss") && w.contains("unsupported data")));
        assert!(warnings.iter().any(|w| w.contains("custom-fixture-entity")), "{warnings:?}");
    }

    #[test]
    fn dwg_unitless_imperial_drawings_keep_their_default_scale() {
        let mut drawing = decode(OLD).unwrap();
        drawing.units = Default::default();
        drawing.props.insert("dwg.measurement".into(), cadkit_core::Value::Int(0));
        let (text, _) = as_dxf(&drawing).unwrap();
        assert_eq!(crate::info(text.as_bytes()).unwrap().unit, Unit::Inches);
        let imported = crate::import(text.as_bytes(), &ImportOptions::default()).unwrap();
        let mut widths = Vec::new();
        imported.document.walk(|node| {
            if matches!(node.kind, NodeKind::Path { .. }) {
                widths.push(node.geometric_bounds().unwrap().width());
            }
        });
        assert!(widths.iter().any(|w| (*w - 7200.0).abs() < 1e-6));
    }
}
