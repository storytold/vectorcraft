//! `document.open` of the files importer plug-ins read ([`vectorcraft_plugins::import`]).
//!
//! A file opens with an importer when `importer` names one (its format id, `plugin.<id>`), which
//! must list the file's extension, or when no built-in format reads the file, by its content or by
//! its extension, and an importer lists its extension (the one whose id sorts first, when several
//! do). A built-in format that can't read a damaged file reports its own error: the file is not
//! handed to an importer instead. Place reads only the built-in formats.
//!
//! The importer is given the file's bytes, at most [`import::MAX_INPUT_BYTES`] of them: a file
//! named by `path` that only an importer can read is read no further than that. Its objects become
//! a new document whose artboard is their bounds, and nothing opens when the importer fails,
//! declines, is cancelled or returns objects that don't pass the checks a filter's output does.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::{Document, MAX_COORD};
use vectorcraft_geom::Rect;
use vectorcraft_plugins::{Plugin, export, import, objects};

use super::super::{Result, bad, str_param};
use super::load::{Loaded, detect, file_name};
use super::{err, extension, file_stem, format};
use crate::EngineError;

const C: &str = "document.open";

/// The importer `document.open`'s params `p` name (`importer`), if any; a value that isn't a
/// string is an error.
fn named(p: &Value) -> Result<Option<&str>> {
    match p.get("importer").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| bad(C, "`importer` must be a plug-in format id (see document.formats)")),
    }
}

/// The importer that opens the file `name` holding `bytes` with `document.open`'s params `p`: the
/// one `importer` names, which must list the file's extension; else, when no built-in format reads
/// the file, the first installed importer that lists its extension; else `None`.
pub(super) fn pick(name: &str, bytes: &[u8], p: &Value) -> Result<Option<Arc<Plugin>>> {
    let ext = extension(name);
    if let Some(id) = named(p)? {
        let plugin = import::named(id).map_err(|e| bad(C, e.to_string()))?;
        let m = plugin.manifest();
        if !m.extensions.contains(&ext) {
            let shown = vectorcraft_plugins::permissions::display_text(&m.name, vectorcraft_plugins::permissions::MAX_SHOWN_NAME);
            let exts: Vec<String> = m.extensions.iter().map(|e| format!(".{e}")).collect();
            return Err(bad(C, format!("“{shown}” opens {} files, not `{}`", exts.join(", "), file_name(name))));
        }
        return Ok(Some(plugin));
    }
    if detect(name, bytes).is_some() {
        return Ok(None);
    }
    Ok(import::for_extension(&ext).into_iter().next())
}

/// How much of a file [`detect`] is given: more than its content checks read.
const SNIFF_BYTES: usize = 64 << 10;

/// Refuses `document.open`'s `dataBase64` in `p` before it is decoded when it holds a file past
/// [`import::MAX_INPUT_BYTES`] that only an importer reads, by the rule of `read_path`.
pub(super) fn check_data(p: &Value) -> Result<()> {
    let Some(b64) = str_param(p, "dataBase64") else {
        return Ok(());
    };
    let name = str_param(p, "name").unwrap_or("Untitled");
    let ext = extension(name);
    let explicit = named(p)?.is_some();
    let built_in = !explicit && (super::readable_format(&ext).is_some() || import::for_extension(&ext).is_empty());
    if built_in || vectorcraft_format::base64_decoded_len(b64) <= import::MAX_INPUT_BYTES as u64 {
        return Ok(());
    }
    if !explicit {
        let start: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).take(SNIFF_BYTES / 3 * 4).collect();
        let head = std::str::from_utf8(&start).ok().and_then(vectorcraft_format::base64_decode);
        if head.is_some_and(|head| detect(name, &head).is_some()) {
            return Ok(());
        }
    }
    Err(err(format!("can't open `{}`: {}", file_name(name), import::too_large())))
}

/// The file at `path` for `document.open`'s params `p`. A file that only an importer reads (one
/// `importer` names, or an extension no built-in format reads that an importer lists) is read up
/// to [`import::MAX_INPUT_BYTES`]: past that, it is an error, unless the start of the file shows a
/// built-in format and no importer was named, which is read whole as any other file is. A file past
/// the limit that isn't read whole is reported from its size, without reading it.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_path(path: &str, p: &Value) -> Result<Vec<u8>> {
    let ext = extension(path);
    let explicit = named(p)?.is_some();
    if !explicit && (super::readable_format(&ext).is_some() || import::for_extension(&ext).is_empty()) {
        return super::read_file(path);
    }
    let head = super::read_file_head(path, SNIFF_BYTES)?;
    if head.len() < SNIFF_BYTES {
        return Ok(head);
    }
    if !explicit && detect(path, &head).is_some() {
        return super::read_file(path);
    }
    let too_large = || err(format!("can't open `{}`: {}", file_name(path), import::too_large()));
    let size = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.len();
    if size > import::MAX_INPUT_BYTES as u64 {
        return Err(too_large());
    }
    // The file can grow after the size is read.
    let file = super::read_file_head(path, import::MAX_INPUT_BYTES + 1)?;
    if file.len() > import::MAX_INPUT_BYTES {
        return Err(too_large());
    }
    Ok(file)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn read_path(path: &str, _: &Value) -> Result<Vec<u8>> {
    super::read_file(path)
}

/// The file `name` holding `bytes` read by importer `plugin` with `document.open`'s params `p`
/// (`params`: the importer's own parameters) into a new document, its artboard the bounds of what
/// the importer returned.
pub(super) fn load(name: &str, bytes: &[u8], plugin: &Plugin, p: &Value) -> Result<Loaded> {
    let m = plugin.manifest();
    let format = format(&export::format_id(&m.id)).filter(|f| f.read).ok_or_else(|| err(vectorcraft_plugins::Error::NotFound(m.id.clone())))?;
    let (ext, stem) = (extension(name), file_stem(name));
    let source = import::Source { extension: &ext, name: &stem };
    let params = p.get("params").cloned().unwrap_or(Value::Null);
    let imported = import::run(plugin, bytes, &params, source).map_err(|e| match e {
        vectorcraft_plugins::Error::Cancelled => EngineError::Cancelled,
        vectorcraft_plugins::Error::Params(why) => bad(C, why),
        e => err(format!("can't open `{}`: {e}", file_name(name))),
    })?;
    let mut doc = Document::new(1.0, 1.0);
    let layer = doc.layers.first().map(|l| l.id);
    let admit = |fx: &[vectorcraft_doc::Effect], n: &vectorcraft_doc::Node| crate::cmd::plugin::admit_effects_for(C, fx, n);
    let ids =
        objects::apply_output(&mut doc, &[], imported.objects, layer, &admit).map_err(|e| err(format!("can't open `{}`: {e}", file_name(name))))?;
    if let (Some(bounds), Some(board)) = (doc.bounds_of(&ids, true), doc.artboards.first_mut()) {
        board.rect = artboard_rect(bounds);
    }
    doc.title = file_name(name);
    Ok(Loaded {
        doc,
        format,
        warnings: imported.warnings,
        restored: false,
        converted: false,
        preview_only: false,
        editing_data: false,
        native_pdf_compatible: None,
    })
}

/// The artboard of art with bounds `r`: `r` within ±[`MAX_COORD`], at least 1 pt wide and high
/// about its center (the artboard of a single line or point). A side that would pass the edge
/// moves inward, so art at the edge keeps an artboard the document's edits accept.
fn artboard_rect(r: Rect) -> Rect {
    let side = |lo: f64, hi: f64| {
        let (lo, hi) = (lo.max(-MAX_COORD), hi.min(MAX_COORD));
        if hi - lo >= 1.0 {
            return (lo, hi);
        }
        let mid = ((lo + hi) / 2.0).clamp(-MAX_COORD + 0.5, MAX_COORD - 0.5);
        (mid - 0.5, mid + 0.5)
    };
    let ((x0, x1), (y0, y1)) = (side(r.x0, r.x1), side(r.y0, r.y1));
    Rect::new(x0, y0, x1, y1)
}
