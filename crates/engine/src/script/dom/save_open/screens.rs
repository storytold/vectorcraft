//! `Document.exportForScreens(exportFolder, exportFormat, options?, itemToExport?, fileNamePrefix?)`
//! with `ExportForScreensItemToExport`, the option classes `ExportForScreensOptionsJPEG`,
//! `ExportForScreensOptionsPNG24`, `ExportForScreensOptionsPNG8`,
//! `ExportForScreensOptionsWebOptimizedSVG` and `ExportForScreensPDFOptions`, and the enumerations
//! they use, as the scripting reference pages
//! <https://www.indesignjs.de/indesignapi/illustrator/Document.html>,
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensItemToExport.html>,
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensOptionsJPEG.html>,
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensOptionsPNG24.html>,
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensOptionsPNG8.html>,
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensOptionsWebOptimizedSVG.html>,
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensPDFOptions.html> and
//! <https://www.indesignjs.de/indesignapi/illustrator/ExportForScreensType.html> describe.
//!
//! The method runs `document.exportForScreens` with one format row into the folder, which goes
//! through the script's file grants as a `File` opened for writing there would, and the command
//! writes only where the script's `File` objects may ([`super::exactly`]).
//! With PNG8 transparency enabled, `matte: false` preserves transparency without a white background. SVG raster images are
//! embedded; `RasterImageLocation.LINK` returns an error.
//!
//! VectorCraft's own rules, which the reference pages do not state:
//! - `exportFolder` is a `Folder`, or a `File` read as the folder at its path. The files go into
//!   that folder, without sub-folders, each named `<fileNamePrefix><name><suffix>.<extension>` as
//!   `document.exportForScreens` names them (`@2x` after a raster file of twice the size), with
//!   the characters a file name may not hold in the prefix written as `-`. The export keeps its
//!   settings in the document, as an export from the dialog does.
//! - `itemToExport` exports, in this order, the artboards of `artboards` (`all`, the default, or a
//!   range of artboard numbers such as `1-3, 5`; `""` for none), the assets of `assets` (their ids)
//!   and, with `document` true, the bounds of the whole document's art (a PDF of every artboard).
//!   One that names nothing throws. Each of the three is its own export: when a later one throws,
//!   the files of the earlier ones stay written.
//! - `ExportForScreensType.SE_JPEG100`, `SE_JPEG80`, `SE_JPEG50` and `SE_JPEG20` write JPEG at a
//!   quality of 100, 80, 50 and 20. A scale factor is from 0.01 to 64 and a resolution from 0.72
//!   to 4608 ppi.
//! - `RasterImageLocation.PRESERVE`, the default, embeds raster images as `EMBED` does, and
//!   `SVGFontType.SVGFONT`, the default, writes text as SVG text elements.
//! - Options are read as `Document.exportFile` reads them: a listed option that is not mapped is
//!   accepted at its default and throws at any other value, and a property the pages do not list
//!   throws an error that names it, unless it is `undefined`. The options object's class must be
//!   the one of `exportFormat`, and an omitted options object takes the defaults a new one holds.

use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostMember};

use super::{
    Answer, Default, Host, Link, MAX_TEXT_BYTES, OptionClass, arg, boolean, builtin_preset_name, current, exactly, file_command, fresh, given,
    member, method, named, num, on_doc, plain, refused, text, this_doc, typename, whole, whole_in, written_as_range,
};

/// The enumerations these members use, as `dom`'s are made.
pub(in crate::script::dom) const ENUMS: &[(&str, &[&str])] = &[
    ("ExportForScreensType", &["SE_JPEG100", "SE_JPEG20", "SE_JPEG50", "SE_JPEG80", "SE_PDF", "SE_PNG24", "SE_PNG8", "SE_SVG"]),
    ("ExportForScreensScaleType", &["SCALEBYFACTOR", "SCALEBYHEIGHT", "SCALEBYRESOLUTION", "SCALEBYWIDTH"]),
    ("AntiAliasingMethod", &["ARTOPTIMIZED", "None", "TYPEOPTIMIZED"]),
    ("JPEGCompressionMethodType", &["BASELINEOPTIMIZED", "BASELINESTANDARD", "PROGRESSIVE"]),
    ("RasterImageLocation", &["EMBED", "LINK", "PRESERVE"]),
    ("SVGCSSPropertyLocation", &["ENTITIES", "PRESENTATIONATTRIBUTES", "STYLEATTRIBUTES", "STYLEELEMENTS"]),
    ("SVGFontType", &["OUTLINEFONT", "SVGFONT"]),
    ("SVGIdType", &["SVGIDMINIMAL", "SVGIDREGULAR", "SVGIDUNIQUE"]),
];

/// The members of `enumeration` in [`ENUMS`].
fn members(enumeration: &str) -> &'static [&'static str] {
    ENUMS.iter().find(|(n, _)| *n == enumeration).map_or(&[][..], |(_, m)| *m)
}

/// The member of `enumeration` that `d` names.
fn pick<'a>(d: &'a Data, enumeration: &str, what: &str) -> std::result::Result<&'a str, HostError> {
    member(d, enumeration, members(enumeration), what)
}

/// The least and most pixels per point a raster file of an export has.
const SCALE_FACTORS: (f64, f64) = (0.01, 64.0);

// ---- Raster options ----

/// The options JPEG, PNG24 and PNG8 share: anti-aliasing and the size, the size kept under `_scale`
/// and `_scaleValue` until [`size`] reads them together.
fn raster_map(class: &str, name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("{class}.{name}");
    match name {
        "antiAliasing" => {
            let mode = match pick(d, "AntiAliasingMethod", &what)? {
                "ARTOPTIMIZED" => "art",
                "TYPEOPTIMIZED" => "type",
                _ => "none",
            };
            p.insert("antiAlias".into(), json!(mode));
        }
        "scaleType" => {
            p.insert("_scale".into(), json!(pick(d, "ExportForScreensScaleType", &what)?));
        }
        "scaleTypeValue" => match d {
            Data::Number(n) if n.is_finite() && *n > 0.0 => {
                p.insert("_scaleValue".into(), json!(n));
            }
            Data::Number(_) => return Err(HostError::Range(format!("{what} must be a number above 0"))),
            _ => return Err(HostError::Type(format!("{what} must be a number"))),
        },
        _ => return Ok(false),
    }
    Ok(true)
}

/// The format row's size from `_scale` and `_scaleValue`: a factor (`scale`), a width or a height
/// in pixels (`width`, `height`), or a resolution (`ppi`).
fn size(p: &mut Map<String, Value>) -> std::result::Result<(), HostError> {
    let how = p.remove("_scale").and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "SCALEBYFACTOR".into());
    let value = p.remove("_scaleValue").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let (lo, hi) = SCALE_FACTORS;
    let (key, per_point) = match how.as_str() {
        "SCALEBYWIDTH" => ("width", None),
        "SCALEBYHEIGHT" => ("height", None),
        "SCALEBYRESOLUTION" => ("ppi", Some(value / 72.0)),
        _ => ("scale", Some(value)),
    };
    if per_point.is_some_and(|f| !(lo..=hi).contains(&f)) {
        let range = if key == "ppi" { "from 0.72 to 4608" } else { "from 0.01 to 64" };
        return Err(HostError::Range(format!("scaleTypeValue must be {range} with ExportForScreensScaleType.{how}")));
    }
    p.insert(key.into(), json!(value));
    Ok(())
}

fn jpeg_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    const CLASS: &str = "ExportForScreensOptionsJPEG";
    let what = format!("{CLASS}.{name}");
    match name {
        "compressionMethod" => {
            let method = match pick(d, "JPEGCompressionMethodType", &what)? {
                "BASELINEOPTIMIZED" => "optimized",
                "PROGRESSIVE" => "progressive",
                _ => "baseline",
            };
            p.insert("method".into(), json!(method));
        }
        "embedICCProfile" => {
            p.insert("embedIcc".into(), json!(boolean(d, &what)?));
        }
        "progressiveScan" => {
            p.insert("scans".into(), json!(whole_in(d, &what, 3, 5)?));
        }
        _ => return raster_map(CLASS, name, d, p),
    }
    Ok(true)
}

fn jpeg_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("antiAliasing", text("AntiAliasingMethod.ARTOPTIMIZED")),
        ("compressionMethod", text("JPEGCompressionMethodType.BASELINESTANDARD")),
        ("embedICCProfile", Data::Bool(false)),
        ("progressiveScan", num(3.0)),
        ("scaleType", text("ExportForScreensScaleType.SCALEBYFACTOR")),
        ("scaleTypeValue", num(1.0)),
    ]
}

/// PNG24's background, kept under `_transparent` and `_black` until [`background`] reads them
/// together.
fn png24_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    const CLASS: &str = "ExportForScreensOptionsPNG24";
    let what = format!("{CLASS}.{name}");
    match name {
        "transparency" => {
            p.insert("_transparent".into(), json!(boolean(d, &what)?));
        }
        "backgroundBlack" => {
            p.insert("_black".into(), json!(boolean(d, &what)?));
        }
        "interlaced" => {
            p.insert("interlaced".into(), json!(boolean(d, &what)?));
        }
        _ => return raster_map(CLASS, name, d, p),
    }
    Ok(true)
}

/// The PNG24 row's `background`: transparent with `transparency`, otherwise black with
/// `backgroundBlack` and white without it.
fn background(p: &mut Map<String, Value>) {
    let transparent = p.remove("_transparent").and_then(|v| v.as_bool()).unwrap_or(true);
    let black = p.remove("_black").and_then(|v| v.as_bool()).unwrap_or(false);
    let background = if transparent {
        "transparent"
    } else if black {
        "black"
    } else {
        "white"
    };
    p.insert("background".into(), json!(background));
}

fn png24_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("antiAliasing", text("AntiAliasingMethod.ARTOPTIMIZED")),
        ("backgroundBlack", Data::Bool(false)),
        ("interlaced", Data::Bool(false)),
        ("scaleType", text("ExportForScreensScaleType.SCALEBYFACTOR")),
        ("scaleTypeValue", num(1.0)),
        ("transparency", Data::Bool(true)),
    ]
}

fn png8_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    const CLASS: &str = "ExportForScreensOptionsPNG8";
    let what = format!("{CLASS}.{name}");
    match name {
        "colorCount" => {
            p.insert("colors".into(), json!(whole_in(d, &what, 2, 256)?));
        }
        "interlaced" => {
            p.insert("interlaced".into(), json!(boolean(d, &what)?));
        }
        "transparency" => {
            p.insert("transparency".into(), json!(boolean(d, &what)?));
        }
        // Matted on white, the default `matteColor`, or not matted.
        "matte" => {
            p.insert("matte".into(), json!(if boolean(d, &what)? { "white" } else { "none" }));
        }
        _ => return raster_map(CLASS, name, d, p),
    }
    Ok(true)
}

const PNG8_KEPT: &[(&str, Default)] = &[("matteColor", Default::Unset)];

fn png8_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("antiAliasing", text("AntiAliasingMethod.ARTOPTIMIZED")),
        ("colorCount", num(128.0)),
        ("interlaced", Data::Bool(false)),
        ("matte", Data::Bool(true)),
        ("scaleType", text("ExportForScreensScaleType.SCALEBYFACTOR")),
        ("scaleTypeValue", num(1.0)),
        ("transparency", Data::Bool(true)),
    ]
}

// ---- Vector options ----

fn svg_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("ExportForScreensOptionsWebOptimizedSVG.{name}");
    let (key, value) = match name {
        "coordinatePrecision" => ("decimals", json!(whole_in(d, &what, 1, 7)?)),
        "cssProperties" => {
            let styling = match pick(d, "SVGCSSPropertyLocation", &what)? {
                "ENTITIES" => "entities",
                "PRESENTATIONATTRIBUTES" => "presentation",
                "STYLEELEMENTS" => "css",
                _ => "style",
            };
            ("styling", json!(styling))
        }
        "fontType" => ("outlineText", json!(pick(d, "SVGFontType", &what)? == "OUTLINEFONT")),
        "rasterImageLocation" => {
            if pick(d, "RasterImageLocation", &what)? == "LINK" {
                return Err(HostError::Error(format!("{what}: RasterImageLocation.LINK is not available yet")));
            }
            ("images", json!("embed"))
        }
        "svgId" => {
            let ids = match pick(d, "SVGIdType", &what)? {
                "SVGIDMINIMAL" => "minimal",
                "SVGIDUNIQUE" => "unique",
                _ => "layerNames",
            };
            ("objectIds", json!(ids))
        }
        "svgMinify" => ("minify", json!(boolean(d, &what)?)),
        "svgResponsive" => ("responsive", json!(boolean(d, &what)?)),
        _ => return Ok(false),
    };
    p.insert(key.into(), value);
    Ok(true)
}

fn svg_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("coordinatePrecision", num(3.0)),
        ("cssProperties", text("SVGCSSPropertyLocation.STYLEATTRIBUTES")),
        ("fontType", text("SVGFontType.SVGFONT")),
        ("rasterImageLocation", text("RasterImageLocation.PRESERVE")),
        ("svgId", text("SVGIdType.SVGIDREGULAR")),
        ("svgMinify", Data::Bool(false)),
        ("svgResponsive", Data::Bool(true)),
    ]
}

/// The PDF preset's name, at most 255 bytes, as the reference page limits it.
fn pdf_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    if name != "pdfPreset" {
        return Ok(false);
    }
    let what = "ExportForScreensPDFOptions.pdfPreset";
    let Data::String(s) = d else { return Err(HostError::Type(format!("{what} must be a string"))) };
    if s.len() > 255 {
        return Err(HostError::Range(format!("{what} is longer than 255 bytes")));
    }
    if !s.is_empty() {
        p.insert("preset".into(), json!(s));
    }
    Ok(true)
}

fn pdf_starts() -> Vec<(&'static str, Data)> {
    Vec::new()
}

// ---- What to export ----

/// `ExportForScreensItemToExport`'s properties, kept under `_artboards`, `_assets` and `_document`
/// until [`exports`] reads them.
fn item_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("ExportForScreensItemToExport.{name}");
    match name {
        "artboards" => {
            let r = match d {
                Data::String(s) if s.len() > MAX_TEXT_BYTES => return Err(HostError::Range(format!("{what} is too long"))),
                Data::String(s) => s.clone(),
                Data::Number(n) => {
                    whole(*n).filter(|n| *n >= 1).ok_or_else(|| HostError::Range(format!("{what} must be a whole number from 1 or a range")))?;
                    vectorcraft_js::number_to_string(*n)
                }
                _ => return Err(HostError::Type(format!("{what} must be a string"))),
            };
            p.insert("_artboards".into(), json!(r));
        }
        "assets" => {
            let Data::Array(ids) = d else { return Err(HostError::Type(format!("{what} must be an array of asset ids"))) };
            let ids = ids
                .iter()
                .map(|id| match id {
                    Data::Number(n) => whole(*n).ok_or_else(|| HostError::Range(format!("{what} holds {n}, not an asset id"))),
                    _ => Err(HostError::Type(format!("{what} must be an array of asset ids"))),
                })
                .collect::<std::result::Result<Vec<u64>, HostError>>()?;
            p.insert("_assets".into(), json!(ids));
        }
        "document" => {
            p.insert("_document".into(), json!(boolean(d, &what)?));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn item_starts() -> Vec<(&'static str, Data)> {
    vec![("artboards", text("all")), ("assets", Data::Array(Vec::new())), ("document", Data::Bool(false))]
}

const JPEG: OptionClass = OptionClass { name: "ExportForScreensOptionsJPEG", map: jpeg_map, kept: &[], starts: jpeg_starts };
const PNG24: OptionClass = OptionClass { name: "ExportForScreensOptionsPNG24", map: png24_map, kept: &[], starts: png24_starts };
const PNG8: OptionClass = OptionClass { name: "ExportForScreensOptionsPNG8", map: png8_map, kept: PNG8_KEPT, starts: png8_starts };
const SVG: OptionClass = OptionClass { name: "ExportForScreensOptionsWebOptimizedSVG", map: svg_map, kept: &[], starts: svg_starts };
const PDF: OptionClass = OptionClass { name: "ExportForScreensPDFOptions", map: pdf_map, kept: &[], starts: pdf_starts };
const ITEM: OptionClass = OptionClass { name: "ExportForScreensItemToExport", map: item_map, kept: &[], starts: item_starts };

/// The params of `class` for `d` (a new object's when `d` is omitted), over a new object's, so a
/// property a record leaves out keeps its default.
fn params(class: &OptionClass, d: Option<&Data>) -> std::result::Result<Map<String, Value>, HostError> {
    let mut p = super::options_params(class, &fresh(class))?;
    if let Some(d) = d {
        p.extend(super::options_params(class, d)?);
    }
    Ok(p)
}

/// The format row `exportFormat` (`d`) asks for, with its option class.
fn format(d: &Data) -> std::result::Result<(Map<String, Value>, &'static OptionClass), HostError> {
    let (format, quality, class) = match pick(d, "ExportForScreensType", "exportFormat")? {
        "SE_JPEG100" => ("jpg", Some(100), &JPEG),
        "SE_JPEG80" => ("jpg", Some(80), &JPEG),
        "SE_JPEG50" => ("jpg", Some(50), &JPEG),
        "SE_JPEG20" => ("jpg", Some(20), &JPEG),
        "SE_PNG24" => ("png", None, &PNG24),
        "SE_PNG8" => ("png8", None, &PNG8),
        "SE_SVG" => ("svg", None, &SVG),
        _ => ("pdf", None, &PDF),
    };
    let mut row = Map::new();
    row.insert("format".into(), json!(format));
    if let Some(q) = quality {
        row.insert("quality".into(), json!(q));
    }
    Ok((row, class))
}

/// The exports `itemToExport` asks for, in order: the artboards, the assets and the whole
/// document, each as the params that pick it.
fn exports(d: Option<&Data>) -> std::result::Result<Vec<Map<String, Value>>, HostError> {
    if let Some(d) = d
        && typename(d) != Some(ITEM.name)
    {
        return Err(HostError::Type("itemToExport must be an ExportForScreensItemToExport object".into()));
    }
    let mut p = params(&ITEM, d)?;
    let mut out = Vec::new();
    if let Some(Value::String(r)) = p.remove("_artboards") {
        let all = r.trim().eq_ignore_ascii_case("all");
        if !r.trim().is_empty() {
            if !all && !written_as_range(&r) {
                return Err(HostError::Range(format!("ExportForScreensItemToExport.artboards `{r}` is not a range of artboard numbers")));
            }
            out.push(Map::from_iter([("range".to_string(), json!(if all { "all" } else { r.as_str() }))]));
        }
    }
    if let Some(ids) = p.remove("_assets").filter(|v| v.as_array().is_some_and(|a| !a.is_empty())) {
        out.push(Map::from_iter([("assets".to_string(), ids)]));
    }
    if p.remove("_document").and_then(|v| v.as_bool()) == Some(true) {
        out.push(Map::from_iter([("fullDocument".to_string(), json!(true))]));
    }
    if out.is_empty() {
        return Err(HostError::Error(
            "exportForScreens has nothing to export: itemToExport names no artboards, no assets and not the document".into(),
        ));
    }
    Ok(out)
}

/// The path of the `Folder` (or `File`) argument `d`, or a `TypeError`.
fn folder_path(d: &Data) -> std::result::Result<&str, HostError> {
    let wrong = || HostError::Type("exportForScreens needs a Folder".into());
    if let Some(p) = vectorcraft_js::file_ref_path(d) {
        return Ok(p);
    }
    let Data::Object(items) = d else { return Err(wrong()) };
    match items.as_slice() {
        [(k, Data::String(path))] if k == vectorcraft_js::FOLDER_REF_KEY => Ok(path),
        _ => Err(wrong()),
    }
}

/// `document.exportForScreens(exportFolder, exportFormat, options?, itemToExport?,
/// fileNamePrefix?)`: `document.exportForScreens` of one format row into the folder, once for each
/// of the artboards, the assets and the whole document that `itemToExport` names.
fn doc_export_for_screens(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let folder = folder_path(arg(a, 1))?;
    let (mut row, class) = format(arg(a, 2))?;
    let options = given(arg(a, 3)).filter(|d| !matches!(d, Data::Null));
    let mut q = params(class, options)?;
    if class.name == PNG24.name {
        background(&mut q);
    }
    if class.name == PDF.name {
        builtin_preset_name(h, &mut q);
    } else if class.name != SVG.name {
        size(&mut q)?;
    }
    row.extend(q);
    let exports = exports(given(arg(a, 4)).filter(|d| !matches!(d, Data::Null)))?;
    let prefix = match given(arg(a, 5)).filter(|d| !matches!(d, Data::Null)) {
        None => None,
        Some(Data::String(s)) if s.len() > MAX_TEXT_BYTES => return Err(HostError::Range("fileNamePrefix is too long".into())),
        Some(Data::String(s)) => Some(s.clone()),
        Some(_) => return Err(HostError::Type("fileNamePrefix must be a string".into())),
    };
    let real = h.disk.borrow().folder_target(folder).map_err(|m| refused(m, folder))?;
    let mut base = Map::new();
    base.insert("folder".into(), json!(plain(real)));
    base.insert("folders".into(), json!("none"));
    base.insert("formats".into(), json!([Value::Object(row)]));
    if let Some(prefix) = prefix {
        base.insert("prefix".into(), json!(prefix));
    }
    // The artboards, the assets and the document are exported one after another into the folder:
    // a later export numbers a file whose name an earlier one took instead of writing over it.
    crate::cmd::fileio::sharing_screen_names(|| {
        for items in exports {
            let mut p = base.clone();
            p.extend(items);
            exactly(h, || on_doc(h, doc.uid, false, |h| file_command(h, "document.exportForScreens", &Value::Object(p))))?;
        }
        Ok(Data::Undefined)
    })
}

/// The members these add to `Document`.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("exportForScreens", method(l, doc_export_for_screens))])
}

/// The classes: each `new Name()` gives a record with its `typename` and the defaults its reference
/// page gives.
pub(super) fn classes(link: &Link) -> Vec<HostClass> {
    [&ITEM, &JPEG, &PNG24, &PNG8, &SVG, &PDF]
        .into_iter()
        .map(|c| {
            let l = link.clone();
            let f: HostFunction = Rc::new(move |_: Vec<Data>| {
                current(&l)?.charge_call(64)?;
                Ok(fresh(c))
            });
            HostClass { name: c.name.into(), parent: None, members: Vec::new(), index: None, construct: Some(f) }
        })
        .collect()
}
