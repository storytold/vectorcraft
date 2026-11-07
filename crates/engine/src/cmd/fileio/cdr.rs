//! CorelDRAW documents (`.cdr`) for every engine path, through `vectorcraft-cdr`:
//!
//! - `document.open` (File › Open) and `file.place` (File › Place) read the file with the `cdr`
//!   options ([`options`]);
//! - `document.export {format: cdr}` (Export As CDR, Export As) writes the version asked for and
//!   validates the written file before handing it over;
//! - `document.cdrInfo` gives the version, the container and whether the whole file reads (for
//!   CorelDRAW Import Options and to check a file), and `document.cdrCapabilities` the
//!   compatibility matrix and the export versions.

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_cdr::{ExportOptions, ImportOptions};
use vectorcraft_doc::Document;

use super::super::{CommandSpec, Result, Session, always, bad, cmd};
use super::{Encoded, FormatOption, err, source};

const C: &str = "document.export";
const INFO: &str = "document.cdrInfo";

/// The CorelDRAW commands: `document.cdrInfo` and `document.cdrCapabilities`.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "document.cdrInfo",
            "CorelDRAW File Info",
            [],
            None,
            "{path} or {name?, dataBase64} the signature, version and container of a CorelDRAW file, read through without opening it → {version, product (CorelDRAW X6…), container (RIFF | ZIP…), supported, note?, readable, pages, layers, objects, skipped, pageSize?: [w, h] mm, warnings, error?}; a file that isn't CDR answers an error",
            always,
            cdr_info
        ),
        cmd!(
            query "document.cdrCapabilities",
            "CorelDRAW Compatibility",
            [],
            None,
            "{} the CDR compatibility matrix → {features: [{feature, import, importNote, export, exportNote}] (levels: full, partial, converted, unsupported), read, exportVersions: [{version, product, container, status: verified|experimental}], defaultExportVersion, verifiedSince}",
            always,
            cdr_capabilities
        ),
    ]
}

/// The options `document.formats` lists for CDR.
pub(super) const OPTIONS: &[FormatOption] = &[
    FormatOption {
        name: "version",
        ty: "integer",
        default: "16",
        description: "CDR format version 7 to 22 (16 = CorelDRAW X6 … 22 = CorelDRAW 2020); 16 and later were verified to open in CorelDRAW, older ones are experimental",
    },
    FormatOption { name: "textToOutlines", ty: "boolean", default: "false", description: "write type as curves instead of editable artistic text" },
    FormatOption {
        name: "validate",
        ty: "boolean",
        default: "true",
        description: "read the written file back and compare it with the document; a file that fails is not written",
    },
    FormatOption {
        name: "rasterizeEffects",
        ty: "boolean",
        default: "false",
        description: "authorize turning raster effects (shadows, glows, blurs) into embedded images; off, they are left out with a warning",
    },
];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ImportParams {
    preserve_scale: Option<bool>,
    scale: Option<f64>,
    preserve_layers: Option<bool>,
    text_to_outlines: Option<bool>,
    warn_missing_fonts: Option<bool>,
    report_unsupported: Option<bool>,
}

/// The CDR import options in `p` (the `cdr` object of `document.open` and `file.place`).
#[derive(Clone, Debug, PartialEq)]
pub struct CdrImport {
    pub options: ImportOptions,
    pub warn_missing_fonts: bool,
}

impl Default for CdrImport {
    fn default() -> Self {
        Self { options: ImportOptions::default(), warn_missing_fonts: true }
    }
}

/// Reads the `cdr` import options of `cmd` from `p`, or the defaults when it is absent or null.
///
/// Fails when `p` isn't an object or the scale isn't a positive number up to 1000.
pub(super) fn options(cmd: &str, p: Option<&Value>) -> Result<CdrImport> {
    let Some(p) = p.filter(|v| !v.is_null()) else { return Ok(CdrImport::default()) };
    if !p.is_object() {
        return Err(bad(cmd, "cdr must be an object of CorelDRAW import options"));
    }
    let q = ImportParams::deserialize(p).map_err(|e| bad(cmd, format!("cdr: {e}")))?;
    let scale = match (q.preserve_scale, q.scale) {
        (Some(false), Some(s)) | (None, Some(s)) => {
            if !(s.is_finite() && s > 0.0 && s <= 1000.0) {
                return Err(bad(cmd, format!("cdr scale must be a positive number up to 1000, not {s}")));
            }
            s
        }
        _ => 1.0,
    };
    let d = ImportOptions::default();
    Ok(CdrImport {
        options: ImportOptions {
            scale,
            preserve_layers: q.preserve_layers.unwrap_or(d.preserve_layers),
            text_to_outlines: q.text_to_outlines.unwrap_or(d.text_to_outlines),
            report_unsupported: q.report_unsupported.unwrap_or(d.report_unsupported),
        },
        warn_missing_fonts: q.warn_missing_fonts.unwrap_or(true),
    })
}

/// A CorelDRAW document and the import notes (what was approximated or left out).
pub(super) fn import(name: &str, bytes: &[u8], o: &CdrImport) -> Result<(Document, Vec<String>)> {
    let title = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let i = vectorcraft_cdr::import(bytes, &title, &o.options).map_err(err)?;
    let mut warnings = Vec::new();
    if let Some(n) = &i.info.note {
        warnings.push(format!("CorelDRAW: {n}"));
    }
    if o.warn_missing_fonts && !o.options.text_to_outlines {
        let db = vectorcraft_text::FontDb::global();
        let missing: Vec<&str> = i.fonts.iter().map(String::as_str).filter(|f| !db.has_family(f)).collect();
        if !missing.is_empty() {
            warnings.push(format!(
                "missing fonts, shown in the fallback font {}: {} (install them, or replace them with Find Font)",
                vectorcraft_text::FALLBACK_FAMILY,
                missing.join(", ")
            ));
        }
    }
    warnings.extend(i.warnings.into_iter().map(|w| format!("CorelDRAW: {w}")));
    Ok((i.document, warnings))
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ExportParams {
    version: u16,
    text_to_outlines: bool,
    validate: bool,
    rasterize_effects: bool,
}

impl Default for ExportParams {
    fn default() -> Self {
        Self { version: vectorcraft_cdr::DEFAULT_EXPORT_VERSION, text_to_outlines: false, validate: true, rasterize_effects: false }
    }
}

/// Checks the CDR export options (the dialog stays open when they fail).
pub fn check(p: &Value) -> std::result::Result<(), String> {
    let q = ExportParams::deserialize(p).map_err(|e| format!("cdr: {e}"))?;
    if !(vectorcraft_cdr::OLDEST_EXPORT_VERSION..=vectorcraft_cdr::NEWEST_EXPORT_VERSION).contains(&q.version) {
        return Err(format!(
            "CDR version {}: choose from {} to {}",
            q.version,
            vectorcraft_cdr::OLDEST_EXPORT_VERSION,
            vectorcraft_cdr::NEWEST_EXPORT_VERSION
        ));
    }
    Ok(())
}

/// The document as CDR, validated.
pub(super) fn encode(doc: &Document, p: &Value) -> Result<Encoded> {
    let q = ExportParams::deserialize(p).map_err(|e| bad(C, format!("cdr: {e}")))?;
    check(p).map_err(|e| bad(C, e))?;
    let flat = q.rasterize_effects.then(|| crate::flatten_raster_effects(doc)).flatten();
    let src = flat.as_ref().unwrap_or(doc);
    let total = 3;
    vectorcraft_doc::cancel::progress(0, total, "Converting the document to CorelDRAW");
    let opts = ExportOptions { version: q.version, text_to_outlines: q.text_to_outlines, validate: q.validate, ..ExportOptions::default() };
    let out = vectorcraft_cdr::export(src, &opts).map_err(|e| bad(C, e.to_string()))?;
    vectorcraft_doc::cancel::progress(total, total, "CorelDRAW file written");
    let mut enc = Encoded::one(out.bytes);
    if out.validation.is_none() {
        enc.warnings.push(format!("CorelDRAW: version {} written without validation", out.version));
    }
    if q.rasterize_effects && flat.is_some() {
        enc.warnings.push("CorelDRAW: raster effects were rasterized into images, as allowed".into());
    }
    enc.warnings.extend(out.warnings.into_iter().map(|w| format!("CorelDRAW: {w}")));
    enc.warnings.extend(crate::cmd::fonts::substitution_warning(src).filter(|_| !q.text_to_outlines));
    Ok(enc)
}

/// Answers the signature, version, container and readability of a CorelDRAW file, reading it through
/// without opening it as a document.
fn cdr_info(_: &mut Session, p: &Value) -> Result<Value> {
    let bytes = source(p, INFO)?.bytes;
    let info = vectorcraft_cdr::detect_version(&bytes).map_err(err)?;
    let v = vectorcraft_cdr::validate(&bytes);
    Ok(json!({
        "version": info.version,
        "product": info.product,
        "container": info.container.label(),
        "supported": info.supported,
        "note": info.note,
        "readable": v.ok(),
        "pages": v.pages,
        "layers": v.layers,
        "objects": v.objects,
        "skipped": v.skipped,
        "pageSize": v.page_size.map(|(w, h)| [w, h]),
        "warnings": v.warnings,
        "error": v.error,
    }))
}

/// Answers the compatibility matrix and the versions the export can write.
fn cdr_capabilities(_: &mut Session, _: &Value) -> Result<Value> {
    Ok(vectorcraft_cdr::matrix::to_json())
}
