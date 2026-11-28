//! The perspective grid of a document and the New Document settings of a startup preset in the
//! script object model, as the help pages <https://ai-scripting.docsforadobe.dev/jsobjref/Document/>
//! and <https://ai-scripting.docsforadobe.dev/jsobjref/Application/> list them, with the values of
//! <https://ai-scripting.docsforadobe.dev/jsobjref/scripting-constants/>.
//!
//! `showPerspectiveGrid`, `hidePerspectiveGrid` and `setActivePlane` change view state with
//! `perspective.grid.show` and `perspective.plane.set`: no undo step, and a failed run puts them
//! back with the document. `selectPerspectivePreset` runs `perspective.grid.preset`, which shows
//! the grid and makes one undo step when the grid's geometry changes. As the Document page
//! describes, `showPerspectiveGrid` and `setActivePlane` give `true` on success, and
//! `hidePerspectiveGrid` gives `true` when it hides a grid and `false` when the grid was already
//! hidden.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `selectPerspectivePreset(gridType, presetName)` throws when no built-in or saved preset has
//!   that name (in any case), and when the preset's grid is not of `gridType`; nothing changes then.
//! - A one-point grid takes only the left and the floor plane as its active plane, as the archived
//!   scripting guide says
//!   (<https://web.archive.org/web/20170622004549/https://wwwimages.adobe.com/content/dam/Adobe/en/devnet/illustrator/sdk/CC2015_3/AI_ScriptGd.pdf>).
//!   `INVALIDGRIDPLANETYPE` makes no plane active.
//! - `showPerspectiveGrid` and `setActivePlane` also give `true` when nothing changed.
//! - `getPresetSettings(preset)` takes the names `documents.addDocument` takes, and gives a new
//!   `DocumentPreset` with the settings a new document from that preset starts with, which
//!   `addDocument` takes back unchanged. Settings it cannot take are left out: units other than
//!   the six `RulerUnits` values, a PPI other than 72, 150 or 300, the artboard count and layout
//!   of more than 100 artboards, and the layout of a grid by column that no row count lays out the
//!   same. Changing the result changes nothing else. Each result counts toward the run's limit
//!   of `DocumentPreset` objects, as `new DocumentPreset()` does.
//! - `startupPresetsList` is an array of the startup preset names, each of which
//!   `documents.addDocument` and `getPresetSettings` take: `Print` first, then the other New
//!   Document profiles, then `Basic CMYK` and `Basic RGB`. Each read gives a new
//!   array; a script that changes that array does not change the list a later read gives.
//! - The workspace methods, `loadColorSettings`, `showPresets` and the perspective grid preset
//!   files are not available yet.

use serde_json::json;
use vectorcraft_js::{Data, HostError, HostMember};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};

use super::{Answer, MAX_TEXT_BYTES, arg, get, later, method, named, num, on_doc, startup_preset, text, this_doc, with_doc};
use crate::script::{Host, Link};

type Result<T> = std::result::Result<T, HostError>;

/// The `PerspectiveGridType` values, with the grid kind (`perspective.grid.set`'s `kind`) of each.
const GRID_TYPES: &[(&str, Option<u8>)] = &[
    ("OnePointPerspectiveGridType", Some(1)),
    ("TwoPointPerspectiveGridType", Some(2)),
    ("ThreePointPerspectiveGridType", Some(3)),
    ("InvalidPerspectiveGridType", None),
];

/// The `PerspectiveGridPlaneType` values, with the plane of each.
const PLANE_TYPES: &[(&str, Plane)] = &[
    ("GRIDLEFTPLANETYPE", Plane::Left),
    ("GRIDRIGHTPLANETYPE", Plane::Right),
    ("GRIDFLOORPLANETYPE", Plane::Ground),
    ("INVALIDGRIDPLANETYPE", Plane::None),
];

/// The enumerations this module adds, as `dom::install` defines them.
pub(super) fn enums() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("PerspectiveGridType", GRID_TYPES.iter().map(|(n, _)| *n).collect()),
        ("PerspectiveGridPlaneType", PLANE_TYPES.iter().map(|(n, _)| *n).collect()),
    ]
}

/// The kind a `PerspectiveGridType` value names (`None`: `InvalidPerspectiveGridType`).
fn grid_type(d: &Data) -> Result<Option<u8>> {
    let found = match d {
        Data::String(s) => s.strip_prefix("PerspectiveGridType.").and_then(|m| GRID_TYPES.iter().find(|(n, _)| *n == m)),
        _ => None,
    };
    found.map(|(_, k)| *k).ok_or_else(|| HostError::Type("gridType must be a PerspectiveGridType value".into()))
}

/// The plane a `PerspectiveGridPlaneType` value names.
fn plane_type(d: &Data) -> Result<Plane> {
    let found = match d {
        Data::String(s) => s.strip_prefix("PerspectiveGridPlaneType.").and_then(|m| PLANE_TYPES.iter().find(|(n, _)| *n == m)),
        _ => None,
    };
    found.map(|(_, p)| *p).ok_or_else(|| HostError::Type("gridPlane must be a PerspectiveGridPlaneType value".into()))
}

/// The document `this` names and its perspective grid.
fn grid(h: &Host, a: &[Data]) -> Result<(super::Doc, PerspectiveGrid)> {
    let doc = this_doc(h, a)?;
    Ok((doc, with_doc(h, doc, |_, st| PerspectiveGrid::effective(&st.doc))))
}

/// `showPerspectiveGrid()`: shows the document's grid (its default one when none was defined).
fn show_grid(h: &Host, a: &[Data]) -> Answer {
    let (doc, g) = grid(h, a)?;
    if !g.visible {
        on_doc(h, doc.uid, false, |h| super::command(h, "perspective.grid.show", &json!({"visible": true})))?;
    }
    Ok(Data::Bool(true))
}

/// `hidePerspectiveGrid()`: hides the document's grid. → whether a grid was hidden.
fn hide_grid(h: &Host, a: &[Data]) -> Answer {
    let (doc, g) = grid(h, a)?;
    if !g.visible {
        return Ok(Data::Bool(false));
    }
    on_doc(h, doc.uid, false, |h| super::command(h, "perspective.grid.show", &json!({"visible": false})))?;
    Ok(Data::Bool(true))
}

/// `getPerspectiveActivePlane()`: the grid's active plane, shown or hidden.
fn active_plane(h: &Host, a: &[Data]) -> Answer {
    let (_, g) = grid(h, a)?;
    let name = PLANE_TYPES.iter().find(|(_, p)| *p == g.plane).map_or("INVALIDGRIDPLANETYPE", |(n, _)| *n);
    Ok(text(format!("PerspectiveGridPlaneType.{name}")))
}

/// `setActivePlane(gridPlane)`: makes `gridPlane` the grid's active plane.
fn set_active_plane(h: &Host, a: &[Data]) -> Answer {
    let plane = plane_type(arg(a, 1))?;
    let (doc, g) = grid(h, a)?;
    if g.kind == 1 && plane == Plane::Right {
        return Err(HostError::Range("a one-point grid has no right plane to make active".into()));
    }
    if g.plane != plane {
        on_doc(h, doc.uid, false, |h| super::command(h, "perspective.plane.set", &json!({"plane": plane.id()})))?;
    }
    Ok(Data::Bool(true))
}

/// `selectPerspectivePreset(gridType, presetName)`: resets the grid to the preset `presetName`,
/// which must be a grid of `gridType`.
fn select_preset(h: &Host, a: &[Data]) -> Answer {
    let kind = grid_type(arg(a, 1))?.ok_or_else(|| HostError::Range("InvalidPerspectiveGridType has no presets".into()))?;
    let Data::String(name) = arg(a, 2) else { return Err(HostError::Type("presetName must be a string".into())) };
    if name.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the preset's name is too long".into()));
    }
    let doc = this_doc(h, a)?;
    let preset = h.session.borrow().perspective_preset(name);
    let preset = preset.ok_or_else(|| HostError::Error(format!("no perspective grid preset called {name}")))?;
    if preset.kind != kind {
        return Err(HostError::Error(format!("the preset {} is not a grid of that type", preset.name)));
    }
    on_doc(h, doc.uid, false, |h| super::command(h, "perspective.grid.preset", &json!({"name": preset.name})))?;
    Ok(Data::Bool(true))
}

/// The `RulerUnits` value of `u`, for the units `documents.addDocument` takes (`None`: other units,
/// which a preset given back to `addDocument` keeps by leaving `units` out).
fn units(u: crate::doc::Unit) -> Option<&'static str> {
    use crate::doc::Unit;
    Some(match u {
        Unit::Points => "RulerUnits.Points",
        Unit::Picas => "RulerUnits.Picas",
        Unit::Inches => "RulerUnits.Inches",
        Unit::Millimeters => "RulerUnits.Millimeters",
        Unit::Centimeters => "RulerUnits.Centimeters",
        Unit::Pixels => "RulerUnits.Pixels",
        _ => return None,
    })
}

/// `artboardRowsOrCols` as `addDocument` reads it: the rows of a grid filled by column and the
/// columns of the other layouts. `None`: a grid by column whose artboards no row count places the
/// same.
fn rows_or_cols(d: &crate::cmd::newdoc::DocSettings) -> Option<usize> {
    use crate::cmd::newdoc::ArtboardLayout;
    let n = d.artboards.max(1) as usize;
    if d.layout != ArtboardLayout::GridByColumn {
        return Some(d.grid_columns().max(1));
    }
    if d.rows > 0 {
        return Some((d.rows as usize).min(n));
    }
    let rows = n.div_ceil(d.grid_columns().max(1));
    let by_rows = crate::cmd::newdoc::DocSettings { rows: u32::try_from(rows).ok()?, ..d.clone() };
    (by_rows.boards() == d.boards()).then_some(rows)
}

/// `app.getPresetSettings(preset)`: a new `DocumentPreset` with the settings a document from the
/// startup preset `preset` starts with.
fn preset_settings(h: &Host, a: &[Data]) -> Answer {
    use crate::cmd::newdoc::{ArtboardLayout, PreviewMode};
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("getPresetSettings needs a startup preset's name".into())) };
    if name.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range("the startup preset's name is too long".into()));
    }
    let (profile, mode) = startup_preset(name).ok_or_else(|| HostError::Error(format!("no startup preset called {name}")))?;
    // The settings `documents.addDocument` starts from: the first preset of the profile's category
    // (found by name, as `file.new` finds it), else New Document's starting settings.
    let d = {
        let s = h.session.borrow();
        let first = profile.and_then(|n| crate::cmd::newdoc::category(&s, n)?.into_iter().next());
        first.map_or_else(|| s.default_settings(), |f| crate::cmd::newdoc::find(&s, &f.name).unwrap_or(f))
    };
    let cmyk = mode.map_or(d.color_mode == crate::doc::ColorMode::Cmyk, |m| m == "cmyk");
    let layout = match (d.layout, d.right_to_left) {
        (ArtboardLayout::GridByRow, false) => "GridByRow",
        (ArtboardLayout::GridByColumn, false) => "GridByCol",
        (ArtboardLayout::Row, false) => "Row",
        (ArtboardLayout::Column, _) => "Column",
        (ArtboardLayout::GridByRow, true) => "RLGridByRow",
        (ArtboardLayout::GridByColumn, true) => "RLGridByCol",
        (ArtboardLayout::Row, true) => "RLRow",
    };
    let preview = match d.preview_mode {
        PreviewMode::Default => "DefaultPreview",
        PreviewMode::Pixel => "PixelPreview",
        PreviewMode::Overprint => "OverprintPreview",
    };
    let item = |k: &str, v: Data| (k.to_string(), v);
    let mut out = vec![
        item("artboardLayout", text(format!("DocumentArtboardLayout.{layout}"))),
        item("artboardSpacing", num(d.spacing)),
        item("colorMode", text(if cmyk { "DocumentColorSpace.CMYK" } else { "DocumentColorSpace.RGB" })),
        item("height", num(d.height)),
        item("previewMode", text(format!("DocumentPreviewMode.{preview}"))),
        item("width", num(d.width)),
    ];
    // The artboard count and `artboardRowsOrCols` are left out when `addDocument` cannot take them
    // back exactly (more than `MAX_ARTBOARDS`, or a grid by column set by its columns that no row
    // count lays out the same), so `addDocument` with the same preset keeps the preset's own.
    if f64::from(d.artboards.max(1)) <= super::MAX_ARTBOARDS {
        out.push(item("numArtboards", num(f64::from(d.artboards.max(1)))));
        if let Some(n) = rows_or_cols(&d) {
            out.push(item("artboardRowsOrCols", num(super::float(n as u64))));
        }
    }
    if let Some(u) = units(d.units) {
        out.push(item("units", text(u)));
    }
    let resolutions = [(72.0, "ScreenResolution"), (150.0, "MediumResolution"), (300.0, "HighResolution")];
    if let Some((_, r)) = resolutions.iter().find(|(ppi, _)| (d.raster_effects_ppi - ppi).abs() < 1e-9) {
        out.push(item("rasterResolution", text(format!("DocumentRasterResolution.{r}"))));
    }
    super::add_preset(h, out)
}

/// The startup presets in the order `app.startupPresetsList` gives them: `Print` first, then the
/// other New Document profiles, then the presets that give a document of one color mode.
const STARTUP_PRESETS: [&str; 9] = ["Print", "Web", "Mobile", "Film & Video", "Art & Illustration", "Branding", "Social", "Basic CMYK", "Basic RGB"];

/// `app.startupPresetsList`: the names of the startup presets, as a new array.
fn startup_presets(_: &Host, _: &[Data]) -> Answer {
    Ok(Data::Array(STARTUP_PRESETS.into_iter().map(text).collect()))
}

/// The members this module adds to `Document`'s.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    let mut m = named(vec![
        ("showPerspectiveGrid", method(l, show_grid)),
        ("hidePerspectiveGrid", method(l, hide_grid)),
        ("getPerspectiveActivePlane", method(l, active_plane)),
        ("setActivePlane", method(l, set_active_plane)),
        ("selectPerspectivePreset", method(l, select_preset)),
    ]);
    m.extend(["exportPerspectiveGridPreset", "importPerspectiveGridPreset"].into_iter().map(|n| later(l, "Document", n)));
    m
}

/// The members this module adds to `Application`'s.
pub(super) fn application_members(l: &Link) -> Vec<(String, HostMember)> {
    let mut m = named(vec![("getPresetSettings", method(l, preset_settings)), ("startupPresetsList", get(l, startup_presets))]);
    let missing = ["saveWorkspace", "switchWorkspace", "deleteWorkspace", "resetWorkspace", "loadColorSettings", "showPresets"];
    m.extend(missing.into_iter().map(|n| later(l, "Application", n)));
    m
}
