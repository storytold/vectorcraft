//! Saving, exporting and opening documents in the script object model: `Document.saveAs`,
//! `Document.exportFile`, `app.open` and `app.userInteractionLevel`, with the option classes the
//! help pages <https://ai-scripting.docsforadobe.dev/jsobjref/Document/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Application/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/IllustratorSaveOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/EPSSaveOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PDFSaveOptions/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/ExportOptionsPNG24/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/ExportOptionsJPEG/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/ExportOptionsSVG/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/PDFFileOptions/> describe.
//!
//! Each method runs a command (`file.saveAs`, `document.export`, `document.open`) inside the run's
//! transaction. The file is a `File` object, and the method reads and writes only where that
//! `File` may ([`super::super::files`]): the path is checked before the command runs, and the
//! command's own reads and writes leave out the files of the open documents that `vc.run`'s
//! commands may use.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - `saveAs` without options saves in the format the path's extension names; it writes the file
//!   at the path as given. Without options, `saveAs` to a `.ai`, `.eps` or `.pdf` path and
//!   `exportFile` as PNG24, JPEG or SVG use the defaults a new options object of that class holds.
//!   `exportFile` adds the format's extension unless the name already ends
//!   with it (`.png`, `.jpg` or `.jpeg`, `.gif`, `.tif` or `.tiff`, `.svg`, `.svgz`), and writes
//!   one file: the active artboard with `artBoardClipping`, otherwise the bounds of the visible art.
//!   An SVG with `embedRasterImages` false also writes the document's embedded images beside it,
//!   and when one of those files may not be written, none of them is.
//! - An option the classes here map is applied; a listed option that is not mapped is accepted at
//!   the default its help page gives and throws an error that says it is not available yet at any
//!   other value; a property the help pages do not list is left out. With `pDFPreset` named, a
//!   `PDFSaveOptions` option still at a new object's default keeps the preset's value.
//! - `app.open(file, documentColorSpace?, options?)` takes `options` as a `PDFFileOptions` record:
//!   an object with `pageToOpen` and `pDFCropToBox`, used for that call only.
//! - `app.userInteractionLevel` starts at `UserInteractionLevel.DISPLAYALERTS` in each run. The
//!   commands a script runs never show a dialog, so the level does not change what they do, and it
//!   never allows a file the script's grants do not.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostMember};

use super::{
    Answer, Host, Link, MAX_TEXT_BYTES, NO_ACTIVE_DOCUMENT, arg, boolean, command, current, doc_ref, given, method, named, num, on_doc, property,
    text, this_doc, whole,
};

/// The most a scale factor of an export takes, in percent.
pub const MAX_SCALE_PERCENT: f64 = 776.19;
/// The least a scale factor of a raster export takes, in percent: `document.export` writes at least
/// 0.01 pixels per point.
pub const MIN_SCALE_PERCENT: f64 = 1.0;

/// What the run keeps for these members: whether `userInteractionLevel` is `DONTDISPLAYALERTS`.
#[derive(Default)]
pub(in crate::script) struct State {
    quiet: Cell<bool>,
}

/// The enumerations these members use, as `dom`'s are made.
pub(super) const ENUMS: &[(&str, &[&str])] = &[
    ("ExportType", &["AutoCAD", "FLASH", "GIF", "JPEG", "PNG24", "PNG8", "SVG", "TIFF"]),
    (
        "Compatibility",
        &[
            "ILLUSTRATOR8",
            "ILLUSTRATOR9",
            "ILLUSTRATOR10",
            "ILLUSTRATOR11",
            "ILLUSTRATOR12",
            "ILLUSTRATOR13",
            "ILLUSTRATOR14",
            "ILLUSTRATOR15",
            "ILLUSTRATOR16",
            "ILLUSTRATOR17",
            "ILLUSTRATOR19",
            "JAPANESEVERSION3",
        ],
    ),
    ("PDFCompatibility", &["ACROBAT4", "ACROBAT5", "ACROBAT6", "ACROBAT7", "ACROBAT8"]),
    ("PDFBoxType", &["PDFARTBOX", "PDFBLEEDBOX", "PDFBOUNDINGBOX", "PDFCROPBOX", "PDFMEDIABOX", "PDFTRIMBOX"]),
    ("EPSPreview", &["BWTIFF", "COLORTIFF", "TRANSPARENTCOLORTIFF", "None"]),
    ("EPSPostScriptLevelEnum", &["LEVEL2", "LEVEL3"]),
    ("PDFOverprint", &["DISCARDPDFOVERPRINT", "PRESERVEPDFOVERPRINT"]),
    ("UserInteractionLevel", &["DISPLAYALERTS", "DONTDISPLAYALERTS"]),
];

const DISPLAY: &str = "UserInteractionLevel.DISPLAYALERTS";
const QUIET: &str = "UserInteractionLevel.DONTDISPLAYALERTS";

/// The default a listed option that is not mapped keeps: a value equal to it is accepted.
#[derive(Clone, Copy)]
enum Default {
    Bool(bool),
    Num(f64),
    Text(&'static str),
    /// An enumerated value, by its enumeration and member (`("DownsampleMethod", "NODOWNSAMPLE")` for
    /// `DownsampleMethod.NODOWNSAMPLE`).
    Member(&'static str, &'static str),
    /// The help page gives no default: any value throws.
    Unset,
}

impl Default {
    fn matches(self, d: &Data) -> bool {
        match (self, d) {
            (Default::Bool(a), Data::Bool(b)) => a == *b,
            (Default::Num(a), Data::Number(b)) => (a - b).abs() < 1e-9,
            (Default::Text(a), Data::String(b)) => a == b,
            (Default::Member(e, m), Data::String(b)) => b.split_once('.').is_some_and(|(be, bm)| be == e && bm == m),
            _ => false,
        }
    }
}

/// A mapped option: from its name and value into the command's params, `Ok(false)` for a name it
/// does not map.
type Mapper = fn(&str, &Data, &mut Map<String, Value>) -> std::result::Result<bool, HostError>;

/// An option class: its name, its mapped options, its listed options that are not mapped, and the
/// defaults a new object starts with.
struct OptionClass {
    name: &'static str,
    map: Mapper,
    kept: &'static [(&'static str, Default)],
    starts: fn() -> Vec<(&'static str, Data)>,
}

fn not_available(class: &str, name: &str) -> HostError {
    HostError::Error(format!("{class}.{name} is not available yet"))
}

fn whole_in(d: &Data, what: &str, lo: u64, hi: u64) -> std::result::Result<u64, HostError> {
    match d {
        Data::Number(n) => {
            whole(*n).filter(|n| (lo..=hi).contains(n)).ok_or_else(|| HostError::Range(format!("{what} must be a whole number from {lo} to {hi}")))
        }
        _ => Err(HostError::Type(format!("{what} must be a number"))),
    }
}

/// The member of `enumeration` that `d` names (`"Enumeration.MEMBER"`), one of `members`.
fn member<'a>(d: &'a Data, enumeration: &str, members: &[&str], what: &str) -> std::result::Result<&'a str, HostError> {
    let bad = || HostError::Type(format!("{what} must be a {enumeration} value"));
    let Data::String(s) = d else { return Err(bad()) };
    let m = s.strip_prefix(enumeration).and_then(|r| r.strip_prefix('.')).ok_or_else(bad)?;
    if members.contains(&m) { Ok(m) } else { Err(bad()) }
}

/// A `Compatibility` value: the versions from 17 on are the file VectorCraft writes; older ones are
/// not available.
fn compatibility(class: &str, d: &Data) -> std::result::Result<(), HostError> {
    let all = ENUMS.iter().find(|(n, _)| *n == "Compatibility").map_or(&[][..], |(_, m)| *m);
    match member(d, "Compatibility", all, &format!("{class}.compatibility"))? {
        "ILLUSTRATOR17" | "ILLUSTRATOR19" => Ok(()),
        _ => Err(not_available(class, "compatibility")),
    }
}

/// An artboard range: a string, given to the command as `range` unless it is empty.
fn range(d: &Data, what: &str, p: &mut Map<String, Value>) -> std::result::Result<(), HostError> {
    let Data::String(s) = d else { return Err(HostError::Type(format!("{what} must be a string"))) };
    if s.len() > MAX_TEXT_BYTES {
        return Err(HostError::Range(format!("{what} is too long")));
    }
    if !s.trim().is_empty() {
        p.insert("range".into(), json!(s));
    }
    Ok(())
}

// ---- Save options ----

fn ai_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("IllustratorSaveOptions.{name}");
    match name {
        "compressed" => p.insert("compress".into(), json!(boolean(d, &what)?)),
        "pdfCompatible" => p.insert("pdfCompatible".into(), json!(boolean(d, &what)?)),
        "embedICCProfile" => p.insert("embedProfiles".into(), json!(boolean(d, &what)?)),
        "compatibility" => {
            compatibility("IllustratorSaveOptions", d)?;
            None
        }
        _ => return Ok(false),
    };
    Ok(true)
}

const AI_KEPT: &[(&str, Default)] = &[
    ("artboardRange", Default::Text("")),
    ("embedLinkedFiles", Default::Bool(false)),
    ("flattenOutput", Default::Member("OutputFlattening", "PRESERVEAPPEARANCE")),
    ("fontSubsetThreshold", Default::Num(100.0)),
    ("saveMultipleArtboards", Default::Bool(false)),
];

fn ai_starts() -> Vec<(&'static str, Data)> {
    vec![("compressed", Data::Bool(true)), ("pdfCompatible", Data::Bool(true)), ("embedICCProfile", Data::Bool(false))]
}

fn eps_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("EPSSaveOptions.{name}");
    match name {
        "cmykPostScript" => {
            p.insert("cmykPostScript".into(), json!(boolean(d, &what)?));
        }
        "includeDocumentThumbnails" => {
            p.insert("thumbnails".into(), json!(boolean(d, &what)?));
        }
        "postScript" => {
            let level = if member(d, "EPSPostScriptLevelEnum", &["LEVEL2", "LEVEL3"], &what)? == "LEVEL2" { 2 } else { 3 };
            p.insert("level".into(), json!(level));
        }
        "preview" => {
            let (format, transparent) = match member(d, "EPSPreview", &["BWTIFF", "COLORTIFF", "TRANSPARENTCOLORTIFF", "None"], &what)? {
                "BWTIFF" => ("tiffBw", None),
                "COLORTIFF" => ("tiffColor", Some(false)),
                "TRANSPARENTCOLORTIFF" => ("tiffColor", Some(true)),
                _ => ("none", None),
            };
            p.insert("previewFormat".into(), json!(format));
            if let Some(t) = transparent {
                p.insert("transparentPreview".into(), json!(t));
            }
        }
        "overprint" => {
            let keep = member(d, "PDFOverprint", &["DISCARDPDFOVERPRINT", "PRESERVEPDFOVERPRINT"], &what)? == "PRESERVEPDFOVERPRINT";
            p.insert("overprints".into(), json!(if keep { "preserve" } else { "discard" }));
        }
        "compatibility" => compatibility("EPSSaveOptions", d)?,
        _ => return Ok(false),
    }
    Ok(true)
}

const EPS_KEPT: &[(&str, Default)] = &[
    ("artboardRange", Default::Text("")),
    ("compatibleGradientPrinting", Default::Bool(false)),
    ("embedAllFonts", Default::Bool(false)),
    ("embedLinkedFiles", Default::Unset),
    ("flattenOuput", Default::Unset),
    ("saveMultipleArtboards", Default::Bool(false)),
];

fn eps_starts() -> Vec<(&'static str, Data)> {
    vec![("postScript", text("EPSPostScriptLevelEnum.LEVEL2")), ("overprint", text("PDFOverprint.PRESERVEPDFOVERPRINT"))]
}

fn pdf_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("PDFSaveOptions.{name}");
    match name {
        "pDFPreset" => {
            let Data::String(s) = d else { return Err(HostError::Type(format!("{what} must be a string"))) };
            if s.len() > MAX_TEXT_BYTES {
                return Err(HostError::Range(format!("{what} is too long")));
            }
            p.insert("preset".into(), json!(s));
        }
        "compatibility" => {
            let version = match member(d, "PDFCompatibility", &["ACROBAT4", "ACROBAT5", "ACROBAT6", "ACROBAT7", "ACROBAT8"], &what)? {
                "ACROBAT4" => "1.3",
                "ACROBAT5" => "1.4",
                "ACROBAT6" => "1.5",
                "ACROBAT7" => "1.6",
                _ => "1.7",
            };
            p.insert("compatibility".into(), json!(version));
        }
        "artboardRange" => range(d, &what, p)?,
        "preserveEditability" => {
            p.insert("preserveEditing".into(), json!(boolean(d, &what)?));
        }
        "acrobatLayers" => {
            p.insert("createLayers".into(), json!(boolean(d, &what)?));
        }
        "compressArt" => {
            p.insert("compression".into(), json!({"compressText": boolean(d, &what)?}));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

const PDF_KEPT: &[(&str, Default)] = &[
    ("bleedLink", Default::Bool(true)),
    ("bleedOffsetRect", Default::Unset),
    ("colorBars", Default::Bool(false)),
    ("colorCompression", Default::Member("CompressionQuality", "None")),
    ("colorConversionID", Default::Member("ColorConversion", "None")),
    ("colorDestinationID", Default::Member("ColorDestination", "None")),
    ("colorDownsampling", Default::Num(150.0)),
    ("colorDownsamplingImageThreshold", Default::Num(225.0)),
    ("colorDownsamplingMethod", Default::Member("DownsampleMethod", "NODOWNSAMPLE")),
    ("colorProfileID", Default::Member("ColorProfile", "None")),
    ("colorTileSize", Default::Num(256.0)),
    ("documentPassword", Default::Text("")),
    ("enableAccess", Default::Bool(true)),
    ("enableCopy", Default::Bool(true)),
    ("enableCopyAccess", Default::Bool(true)),
    ("enablePlainText", Default::Bool(false)),
    ("flattenerOptions", Default::Unset),
    ("flattenerPreset", Default::Unset),
    ("fontSubsetThreshold", Default::Num(100.0)),
    ("generateThumbnails", Default::Bool(true)),
    ("grayscaleCompression", Default::Member("CompressionQuality", "None")),
    ("grayscaleDownsampling", Default::Num(150.0)),
    ("grayscaleDownsamplingImageThreshold", Default::Num(225.0)),
    ("grayscaleDownsamplingMethod", Default::Member("DownsampleMethod", "NODOWNSAMPLE")),
    ("grayscaleTileSize", Default::Num(256.0)),
    ("monochromeCompression", Default::Member("MonochromeCompression", "None")),
    ("monochromeDownsampling", Default::Num(300.0)),
    ("monochromeDownsamplingImageThreshold", Default::Num(450.0)),
    ("monochromeDownsamplingMethod", Default::Member("DownsampleMethod", "NODOWNSAMPLE")),
    ("offset", Default::Num(0.0)),
    ("optimization", Default::Bool(false)),
    ("outputCondition", Default::Unset),
    ("outputConditionID", Default::Unset),
    ("pageInformation", Default::Bool(false)),
    ("pageMarksType", Default::Member("PageMarksTypes", "Roman")),
    ("pDFAllowPrinting", Default::Member("PDFPrintAllowedEnum", "PRINT128HIGHRESOLUTION")),
    ("pDFChangesAllowed", Default::Member("PDFChangesAllowedEnum", "CHANGE128ANYCHANGES")),
    ("pDFXStandard", Default::Member("PDFXStandard", "PDFXNONE")),
    ("pDFXStandardDescription", Default::Unset),
    ("permissionPassword", Default::Text("")),
    ("printerResolution", Default::Num(800.0)),
    ("registrationMarks", Default::Bool(false)),
    ("requireDocumentPassword", Default::Bool(false)),
    ("requirePermissionPassword", Default::Bool(false)),
    ("trapped", Default::Bool(false)),
    ("trimMarks", Default::Bool(false)),
    ("trimMarkWeight", Default::Member("PDFTrimMarkWeight", "TRIMMARKWEIGHT0125")),
    ("viewAfterSaving", Default::Bool(false)),
];

fn pdf_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("compatibility", text("PDFCompatibility.ACROBAT5")),
        ("artboardRange", text("")),
        ("preserveEditability", Data::Bool(true)),
        ("acrobatLayers", Data::Bool(false)),
        ("compressArt", Data::Bool(true)),
    ]
}

const SAVE_CLASSES: &[(OptionClass, &str)] = &[
    (OptionClass { name: "IllustratorSaveOptions", map: ai_map, kept: AI_KEPT, starts: ai_starts }, "ai"),
    (OptionClass { name: "EPSSaveOptions", map: eps_map, kept: EPS_KEPT, starts: eps_starts }, "eps"),
    (OptionClass { name: "PDFSaveOptions", map: pdf_map, kept: PDF_KEPT, starts: pdf_starts }, "pdf"),
];

// ---- Export options ----

/// The raster options PNG24 and JPEG share: anti-aliasing, the clip to the artboard and the scale.
fn raster_map(class: &str, name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("{class}.{name}");
    match name {
        "antiAliasing" => {
            p.insert("antiAlias".into(), json!(if boolean(d, &what)? { "art" } else { "none" }));
        }
        "artBoardClipping" => {
            p.insert("_clip".into(), json!(boolean(d, &what)?));
        }
        "horizontalScale" | "verticalScale" => {
            let n = match d {
                Data::Number(n) if n.is_finite() => *n,
                _ => return Err(HostError::Type(format!("{what} must be a finite number"))),
            };
            if !(MIN_SCALE_PERCENT..=MAX_SCALE_PERCENT).contains(&n) {
                return Err(HostError::Range(format!("{what} must be at least 1 and at most 776.19")));
            }
            let key = if name == "horizontalScale" { "_h" } else { "_v" };
            p.insert(key.into(), json!(n));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn png24_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    if name == "transparency" {
        let clear = boolean(d, "ExportOptionsPNG24.transparency")?;
        p.insert("background".into(), json!(if clear { "transparent" } else { "white" }));
        return Ok(true);
    }
    raster_map("ExportOptionsPNG24", name, d, p)
}

const PNG24_KEPT: &[(&str, Default)] = &[("matte", Default::Bool(true)), ("matteColor", Default::Unset), ("saveAsHTML", Default::Bool(false))];

fn png24_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("antiAliasing", Data::Bool(true)),
        ("artBoardClipping", Data::Bool(false)),
        ("horizontalScale", num(100.0)),
        ("verticalScale", num(100.0)),
        ("transparency", Data::Bool(true)),
    ]
}

fn jpeg_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    match name {
        "qualitySetting" => {
            p.insert("quality".into(), json!(whole_in(d, "ExportOptionsJPEG.qualitySetting", 0, 100)?));
            Ok(true)
        }
        "optimization" => {
            let optimized = boolean(d, "ExportOptionsJPEG.optimization")?;
            p.insert("method".into(), json!(if optimized { "optimized" } else { "baseline" }));
            Ok(true)
        }
        _ => raster_map("ExportOptionsJPEG", name, d, p),
    }
}

const JPEG_KEPT: &[(&str, Default)] =
    &[("blurAmount", Default::Num(0.0)), ("matte", Default::Bool(true)), ("matteColor", Default::Unset), ("saveAsHTML", Default::Bool(false))];

fn jpeg_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("antiAliasing", Data::Bool(true)),
        ("horizontalScale", num(100.0)),
        ("verticalScale", num(100.0)),
        ("optimization", Data::Bool(true)),
        ("qualitySetting", num(30.0)),
    ]
}

fn svg_map(name: &str, d: &Data, p: &mut Map<String, Value>) -> std::result::Result<bool, HostError> {
    let what = format!("ExportOptionsSVG.{name}");
    match name {
        "coordinatePrecision" => {
            p.insert("decimals".into(), json!(whole_in(d, &what, 1, 7)?));
        }
        "embedRasterImages" => {
            p.insert("images".into(), json!(if boolean(d, &what)? { "embed" } else { "link" }));
        }
        "compressed" => {
            p.insert("_svgz".into(), json!(boolean(d, &what)?));
        }
        "preserveEditability" => {
            p.insert("preserveEditing".into(), json!(boolean(d, &what)?));
        }
        "includeVariablesAndDatasets" => {
            p.insert("variableData".into(), json!(boolean(d, &what)?));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

const SVG_KEPT: &[(&str, Default)] = &[
    ("artboardRange", Default::Text("")),
    ("cssProperties", Default::Member("SVGCSSPropertyLocation", "STYLEATTRIBUTES")),
    ("documentEncoding", Default::Member("SVGDocumentEncoding", "ASCII")),
    ("DTD", Default::Member("SVGDTDVersion", "SVG1_1")),
    ("fontSubsetting", Default::Member("SVGFontSubsetting", "ALLGLYPHS")),
    ("fontType", Default::Member("SVGFontType", "CEFFONT")),
    ("includeFileInfo", Default::Bool(false)),
    ("includeUnusedStyles", Default::Bool(false)),
    ("optimizeForSVGViewer", Default::Bool(false)),
    ("saveMultipleArtboards", Default::Bool(false)),
    ("slices", Default::Bool(false)),
    ("sVGAutoKerning", Default::Bool(false)),
    ("sVGTextOnPath", Default::Bool(false)),
];

fn svg_starts() -> Vec<(&'static str, Data)> {
    vec![
        ("compressed", Data::Bool(false)),
        ("coordinatePrecision", num(3.0)),
        ("embedRasterImages", Data::Bool(false)),
        ("includeVariablesAndDatasets", Data::Bool(false)),
        ("preserveEditability", Data::Bool(false)),
    ]
}

const PNG24: OptionClass = OptionClass { name: "ExportOptionsPNG24", map: png24_map, kept: PNG24_KEPT, starts: png24_starts };
const JPEG: OptionClass = OptionClass { name: "ExportOptionsJPEG", map: jpeg_map, kept: JPEG_KEPT, starts: jpeg_starts };
const SVG: OptionClass = OptionClass { name: "ExportOptionsSVG", map: svg_map, kept: SVG_KEPT, starts: svg_starts };

/// An `ExportType`: the format `document.export` writes, the extensions a file name may already
/// end with (the first is added), and its option class (`None`: no options are taken).
struct ExportKind {
    format: &'static str,
    extensions: &'static [&'static str],
    options: Option<&'static OptionClass>,
}

fn export_kind(d: &Data) -> std::result::Result<ExportKind, HostError> {
    let all = ENUMS.iter().find(|(n, _)| *n == "ExportType").map_or(&[][..], |(_, m)| *m);
    let raster = |format, extensions| ExportKind { format, extensions, options: None };
    Ok(match member(d, "ExportType", all, "exportType")? {
        "PNG24" => ExportKind { format: "png", extensions: &["png"], options: Some(&PNG24) },
        "JPEG" => ExportKind { format: "jpg", extensions: &["jpg", "jpeg"], options: Some(&JPEG) },
        "SVG" => ExportKind { format: "svg", extensions: &["svg"], options: Some(&SVG) },
        "PNG8" => raster("png8", &["png"]),
        "GIF" => raster("gif", &["gif"]),
        "TIFF" => raster("tiff", &["tif", "tiff"]),
        other => return Err(HostError::Error(format!("ExportType.{other} is not available yet"))),
    })
}

/// The command's params for the options object `d` of `class`, read as the module's rules say.
fn options_params(class: &OptionClass, d: &Data) -> std::result::Result<Map<String, Value>, HostError> {
    let wrong = || HostError::Type(format!("the options must be an {} object", class.name));
    let Data::Object(items) = d else { return Err(wrong()) };
    if !items.iter().any(|(k, v)| k == "typename" && matches!(v, Data::String(s) if s == class.name)) {
        return Err(wrong());
    }
    // With a PDF preset named, an option still at a new object's default leaves the preset's value.
    let preset = class.name == "PDFSaveOptions" && items.iter().any(|(k, v)| k == "pDFPreset" && matches!(v, Data::String(s) if !s.is_empty()));
    let starts = if preset { (class.starts)() } else { Vec::new() };
    let mut p = Map::new();
    for (k, v) in items {
        if k == "typename" || matches!(v, Data::Undefined) || starts.iter().any(|(n, s)| n == k && s == v) {
            continue;
        }
        if (class.map)(k, v, &mut p)? {
            continue;
        }
        match class.kept.iter().find(|(n, _)| n == k) {
            Some((_, default)) if default.matches(v) => {}
            Some(_) => return Err(not_available(class.name, k)),
            None => {}
        }
    }
    Ok(p)
}

/// What `new Name()` gives for `class`: its `typename` and the defaults its help page gives for the
/// options mapped here.
fn fresh(class: &OptionClass) -> Data {
    let mut items = vec![("typename".to_string(), text(class.name))];
    items.extend((class.starts)().into_iter().map(|(k, v)| (k.to_string(), v)));
    Data::Object(items)
}

/// The typename of an options object, when it has one.
fn typename(d: &Data) -> Option<&str> {
    let Data::Object(items) = d else { return None };
    items.iter().find_map(|(k, v)| match (k.as_str(), v) {
        ("typename", Data::String(s)) => Some(s.as_str()),
        _ => None,
    })
}

// ---- Files ----

/// The path of the `File` argument `d`, or a `TypeError` naming the method.
fn file_path<'a>(d: &'a Data, method: &str) -> std::result::Result<&'a str, HostError> {
    vectorcraft_js::file_ref_path(d).ok_or_else(|| HostError::Type(format!("{method} needs a File")))
}

/// The error a refused path throws: the `File` error and the path.
fn refused(message: &str, path: &str) -> HostError {
    HostError::Error(format!("{message}: {path}"))
}

/// Runs `f` with the commands' reads and writes held to the script's own grants
/// ([`super::super::files::Disk::exact`]).
fn exactly(h: &Host, f: impl FnOnce() -> Answer) -> Answer {
    h.disk.borrow_mut().exact = true;
    let r = f();
    h.disk.borrow_mut().exact = false;
    r
}

/// The path to give a command for the real path `real`.
fn plain(real: std::path::PathBuf) -> String {
    crate::cmd::findfiles::plain(real).to_string_lossy().into_owned()
}

/// `document.saveAs(file, options?)`: `file.saveAs` on the document, in the format the options'
/// class names (`ai`, `eps`, `pdf`), or without options in the format of the path's extension.
fn doc_save_as(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let path = file_path(arg(a, 1), "saveAs")?;
    let mut p = Map::new();
    let given_options = given(arg(a, 2)).filter(|d| !matches!(d, Data::Null));
    // Without options, a path ending in `.ai`, `.eps` or `.pdf` takes the defaults a new options
    // object of that format's class holds.
    let extension = path.rsplit(['/', '\\']).next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase());
    let by_extension = SAVE_CLASSES.iter().find(|(_, f)| Some(*f) == extension.as_deref()).map(|(c, _)| fresh(c));
    if let Some(o) = given_options.or(by_extension.as_ref()) {
        let name = typename(o);
        let Some((class, format)) = SAVE_CLASSES.iter().find(|(c, _)| Some(c.name) == name) else {
            return Err(HostError::Type("saveAs options must be IllustratorSaveOptions, EPSSaveOptions or PDFSaveOptions".into()));
        };
        let q = options_params(class, o)?;
        p.insert("format".into(), json!(format));
        if *format == "ai" {
            p.extend(q);
        } else if !q.is_empty() {
            p.insert("options".into(), Value::Object(q));
        }
    }
    let real = h.disk.borrow().save_target(path).map_err(|m| refused(m, path))?;
    p.insert("path".into(), json!(plain(real)));
    exactly(h, || on_doc(h, doc.uid, false, |h| command(h, "file.saveAs", &Value::Object(p))))?;
    Ok(Data::Undefined)
}

/// `path` with the extension of `kind` added, unless its name already ends with one of them.
fn with_extension(path: &str, kind: &ExportKind, svgz: bool) -> String {
    let extensions: &[&str] = if svgz { &["svgz"] } else { kind.extensions };
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_lowercase();
    match extensions.first() {
        Some(first) if !extensions.iter().any(|e| name.ends_with(&format!(".{e}"))) => format!("{path}.{first}"),
        _ => path.to_string(),
    }
}

/// `document.exportFile(file, exportType, options?)`: `document.export` of one file, which leaves
/// the document's path as it is.
fn doc_export_file(h: &Host, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    let path = file_path(arg(a, 1), "exportFile")?;
    let kind = export_kind(arg(a, 2))?;
    // Without options, an export takes the defaults a new options object of its class holds.
    let mut p = match given(arg(a, 3)).filter(|d| !matches!(d, Data::Null)) {
        None => match kind.options {
            Some(class) => options_params(class, &fresh(class))?,
            None => Map::new(),
        },
        Some(o) => match kind.options {
            Some(class) => options_params(class, o)?,
            None => return Err(HostError::Error(format!("options for {} are not available yet", kind.format))),
        },
    };
    let svgz = p.remove("_svgz").and_then(|v| v.as_bool()).unwrap_or(false);
    let clip = p.remove("_clip").and_then(|v| v.as_bool()).unwrap_or(false);
    let (hs, vs) = (p.remove("_h").and_then(|v| v.as_f64()).unwrap_or(100.0), p.remove("_v").and_then(|v| v.as_f64()).unwrap_or(100.0));
    if (hs - vs).abs() > 1e-9 {
        return Err(HostError::Error("different horizontalScale and verticalScale are not available yet".into()));
    }
    // A raster file has one pixel per point at 100%, whatever the document's raster effects resolution.
    if kind.format != "svg" {
        p.insert("scale".into(), json!(hs / 100.0));
    }
    let format = if svgz { "svgz" } else { kind.format };
    p.insert("format".into(), json!(format));
    if kind.format == "svg" {
        p.insert("useArtboards".into(), json!(false));
    } else if clip {
        let artboard = super::with_doc(h, doc, |_, st| super::active_artboard(st));
        p.insert("artboard".into(), json!(artboard));
    } else {
        p.insert("useArtboards".into(), json!(false));
    }
    let named = with_extension(path, &kind, svgz);
    let real = h.disk.borrow().save_target(&named).map_err(|m| refused(m, &named))?;
    p.insert("path".into(), json!(plain(real)));
    exactly(h, || on_doc(h, doc.uid, false, |h| command(h, "document.export", &Value::Object(p))))?;
    Ok(Data::Undefined)
}

/// `app.open(file, documentColorSpace?, options?)`: `document.open` of the file, which becomes
/// the active document. → the document.
fn app_open(h: &Host, a: &[Data]) -> Answer {
    let path = file_path(arg(a, 1), "open")?;
    let mut p = Map::new();
    if let Some(d) = given(arg(a, 2)).filter(|d| !matches!(d, Data::Null)) {
        let mode = if member(d, "DocumentColorSpace", &["CMYK", "RGB"], "documentColorSpace")? == "RGB" { "rgb" } else { "cmyk" };
        p.insert("colorMode".into(), json!(mode));
    }
    if let Some(o) = given(arg(a, 3)).filter(|d| !matches!(d, Data::Null)) {
        let Data::Object(items) = o else { return Err(HostError::Type("open's options must be a PDFFileOptions record".into())) };
        if typename(o).is_some_and(|t| t != "PDFFileOptions") {
            return Err(HostError::Type("open's options must be a PDFFileOptions record".into()));
        }
        for (k, v) in items {
            match k.as_str() {
                "pageToOpen" => {
                    p.insert("page".into(), json!(whole_in(v, "PDFFileOptions.pageToOpen", 1, 1_000_000)?));
                }
                "pDFCropToBox" => {
                    let boxes = ["PDFARTBOX", "PDFBLEEDBOX", "PDFBOUNDINGBOX", "PDFCROPBOX", "PDFMEDIABOX", "PDFTRIMBOX"];
                    let crop = match member(v, "PDFBoxType", &boxes, "PDFFileOptions.pDFCropToBox")? {
                        "PDFARTBOX" => "art",
                        "PDFBLEEDBOX" => "bleed",
                        "PDFBOUNDINGBOX" => "bounding",
                        "PDFCROPBOX" => "crop",
                        "PDFMEDIABOX" => "media",
                        _ => "trim",
                    };
                    p.insert("cropTo".into(), json!(crop));
                }
                _ => {}
            }
        }
    }
    let real = h.disk.borrow().open_target(path).map_err(|m| refused(m, path))?;
    p.insert("path".into(), json!(plain(real)));
    let r = exactly(h, || command(h, "document.open", &Value::Object(p)))?;
    let index = match &r {
        Data::Object(items) => items.iter().find_map(|(k, v)| match (k.as_str(), v) {
            ("index", Data::Number(n)) => whole(*n).and_then(|i| usize::try_from(i).ok()),
            _ => None,
        }),
        _ => None,
    };
    let s = h.session.borrow();
    let st = index.and_then(|i| s.documents().get(i)).or_else(|| s.active()).ok_or_else(|| HostError::Error(NO_ACTIVE_DOCUMENT.into()))?;
    Ok(doc_ref(h, st.uid))
}

fn app_interaction(h: &Host, _: &[Data]) -> Answer {
    Ok(text(if h.dom.save_options.quiet.get() { QUIET } else { DISPLAY }))
}

fn app_set_interaction(h: &Host, a: &[Data]) -> Answer {
    let quiet = member(arg(a, 1), "UserInteractionLevel", &["DISPLAYALERTS", "DONTDISPLAYALERTS"], "userInteractionLevel")? == "DONTDISPLAYALERTS";
    h.dom.save_options.quiet.set(quiet);
    Ok(Data::Undefined)
}

/// The members these add to `Application`.
pub(super) fn application_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("open", method(l, app_open)), ("userInteractionLevel", property(l, app_interaction, app_set_interaction))])
}

/// The members these add to `Document`.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("saveAs", method(l, doc_save_as)), ("exportFile", method(l, doc_export_file))])
}

/// The option classes: each `new Name()` gives a record with its `typename` and the defaults its
/// help page gives for the options mapped here.
pub(super) fn classes(link: &Link) -> Vec<HostClass> {
    let all = SAVE_CLASSES.iter().map(|(c, _)| c).chain([&PNG24, &JPEG, &SVG]);
    all.map(|c| {
        let l = link.clone();
        let f: HostFunction = Rc::new(move |_: Vec<Data>| {
            current(&l)?.charge_call(64)?;
            Ok(fresh(c))
        });
        HostClass { name: c.name.into(), parent: None, members: Vec::new(), index: None, construct: Some(f) }
    })
    .collect()
}
