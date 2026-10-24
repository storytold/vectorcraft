//! The CDR compatibility matrix: what each feature becomes on import and on export. It feeds the
//! `document.cdrCapabilities` command and `docs/cdr-compatibility.md`. A lossy conversion is never
//! listed as full support.

use serde::Serialize;

/// How well a feature is supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    /// Fully supported: kept and editable.
    Full,
    /// Partially supported: the essentials are kept, some attributes are lost.
    Partial,
    /// Converted with limitations: becomes another editable representation (curves, groups, a
    /// solid colour).
    Converted,
    /// Not supported: left out, with a warning.
    Unsupported,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Fully supported",
            Self::Partial => "Partially supported",
            Self::Converted => "Converted with limitations",
            Self::Unsupported => "Not supported",
        }
    }
}

/// A feature and how it is handled each way.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    pub feature: &'static str,
    pub import: Level,
    pub import_note: &'static str,
    pub export: Level,
    pub export_note: &'static str,
}

use Level::*;

/// The matrix.
pub const FEATURES: &[Feature] = &[
    Feature {
        feature: "Vector paths and Bézier curves",
        import: Full,
        import_note: "Nodes, cubic curves and closed subpaths kept as editable paths (even-odd fill, as CorelDRAW fills them)",
        export: Full,
        export_note: "Written as CDR curves with their nodes; node types (cusp, smooth, symmetrical) come from the handles",
    },
    Feature {
        feature: "Shapes and geometric primitives",
        import: Converted,
        import_note: "Rectangles (with corners), ellipses, arcs, polygons and stars become editable paths of the same geometry; the primitive's parameters are lost",
        export: Converted,
        export_note: "VectorCraft live shapes are written as curves",
    },
    Feature {
        feature: "Positioning and transformations",
        import: Full,
        import_note: "Each object's matrix is applied to its geometry; type and images keep their matrix",
        export: Full,
        export_note: "Coordinates converted from points to CDR units relative to the page; type and images keep their own matrix",
    },
    Feature {
        feature: "Groups and hierarchy",
        import: Full,
        import_note: "Nested groups kept (up to 64 levels); tables and symbols become groups",
        export: Partial,
        export_note: "Groups kept; sublayers become groups; symbols, blends and repeats become the art they draw",
    },
    Feature {
        feature: "Layers and layer order",
        import: Partial,
        import_note: "Name and order; master layers under the first page. The reader doesn't read visibility, lock or print settings: layers come in visible and unlocked",
        export: Partial,
        export_note: "Name and order; hidden layers and objects are left out (with a warning), as the writer keeps no hidden state; lock isn't written; layer opacity moves to the objects; template layers are left out",
    },
    Feature {
        feature: "Text content and font attributes",
        import: Partial,
        import_note: "Editable artistic and paragraph text: family, bold/italic, size, colour, underline, strikethrough, alignment, tracking. Type on a path loses its path; leading and paragraph indents use the defaults",
        export: Partial,
        export_note: "Point type written as editable artistic text (verified in CorelDRAW from version 16) or as outlines on request; area, vertical and path type, and type inside clipping groups, become outlines",
    },
    Feature {
        feature: "Fill colours and stroke properties",
        import: Full,
        import_note: "RGB, CMYK, greyscale and Lab colours; strokes with width, caps, joins and dashes",
        export: Partial,
        export_note: "One fill and one stroke per object (the first of each); inside/outside alignment, arrowheads and width profiles are lost; Lab becomes RGB",
    },
    Feature {
        feature: "Gradients",
        import: Converted,
        import_note: "Linear and radial with their colour stops; conical, square and shaped become radial; repeats and mirroring are expanded into stops",
        export: Converted,
        export_note: "Linear and radial with angle and centre; stop opacity and midpoints are lost; freeform becomes linear",
    },
    Feature {
        feature: "Transparency",
        import: Partial,
        import_note: "An object's uniform transparency becomes its opacity; gradient transparencies don't",
        export: Partial,
        export_note: "Object, fill and group opacity combined into one uniform transparency per object",
    },
    Feature {
        feature: "Clipping paths and masks",
        import: Partial,
        import_note: "PowerClip becomes an editable clipping group; opacity masks are not imported",
        export: Converted,
        export_note: "The clip is applied to the geometry (a boolean intersection, editable; clipped strokes become filled areas); clipped bitmaps get a native crop path; opacity masks and blending modes are not written",
    },
    Feature {
        feature: "Page dimensions and document units",
        import: Full,
        import_note: "Exact physical size (mm → pt); the document in millimetres",
        export: Full,
        export_note: "Each artboard becomes a page of the same size; checked by the validation",
    },
    Feature {
        feature: "Multiple pages",
        import: Full,
        import_note: "One artboard per page, side by side, with each page's layers",
        export: Full,
        export_note: "One page per artboard; each object goes to the artboard holding its centre",
    },
    Feature {
        feature: "Embedded bitmaps",
        import: Full,
        import_note: "Pixels kept (RGB, CMYK converted, transparency) with position and transformation",
        export: Partial,
        export_note: "Images embedded at 24 bits; linked images are embedded; bitmap alpha may be lost",
    },
    Feature {
        feature: "Colour profiles and CMYK",
        import: Partial,
        import_note: "CMYK values kept; the file's ICC profile is not applied (the active one is); spot colours become CMYK",
        export: Partial,
        export_note: "CMYK and RGB values kept; ICC profiles and spot colours are not written",
    },
    Feature {
        feature: "Effects (shadows, blends, extrusions, lenses, envelopes)",
        import: Unsupported,
        import_note: "Live effects are not recreated; the shapes stored in the file are; shadows are left out (with a warning)",
        export: Converted,
        export_note: "Live vector effects are applied and written as shapes; raster effects are left out unless rasterizing them is allowed",
    },
    Feature {
        feature: "Pattern, texture and mesh fills",
        import: Converted,
        import_note: "Replaced by an approximate solid colour (with a warning)",
        export: Converted,
        export_note: "Patterns written in their first solid colour; gradient meshes written as the art they draw",
    },
];

/// The versions read.
pub const READ_NOTE: &str =
    "Read: versions 3 to 26 (RIFF up to 13; ZIP with riffData.cdr in 14 and 15; ZIP with root.dat from 16). Support before version 6 is partial.";

/// The matrix as a Markdown table (for the documentation).
pub fn markdown() -> String {
    let mut s = String::from("| Feature | Import | Export |\n|---|---|---|\n");
    for f in FEATURES {
        s.push_str(&format!("| {} | **{}**: {} | **{}**: {} |\n", f.feature, f.import.label(), f.import_note, f.export.label(), f.export_note));
    }
    s
}

/// The matrix and the export versions as JSON.
pub fn to_json() -> serde_json::Value {
    let versions: Vec<serde_json::Value> = crate::export_versions()
        .into_iter()
        .map(|v| {
            serde_json::json!({
                "version": v.version,
                "product": v.product,
                "container": v.container.label(),
                "status": match v.status {
                    crate::ExportStatus::Verified => "verified",
                    crate::ExportStatus::Experimental => "experimental",
                },
            })
        })
        .collect();
    serde_json::json!({
        "features": FEATURES,
        "read": READ_NOTE,
        "exportVersions": versions,
        "defaultExportVersion": crate::DEFAULT_EXPORT_VERSION,
        "verifiedSince": crate::VERIFIED_SINCE,
        "levels": ["full", "partial", "converted", "unsupported"],
    })
}
