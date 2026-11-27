//! `document.open` and `file.place` of the files importer plug-ins read
//! ([`vectorcraft_plugins::import`]).
//!
//! A file opens with an importer when `importer` names one (its format id, `plugin.<id>`), which
//! must list the file's extension, or when no built-in format reads the file, by its content or by
//! its extension, and an importer lists its extension (the one whose id sorts first, when several
//! do). A built-in format that can't read a damaged file reports its own error: the file is not
//! handed to an importer instead. File › Place uses an importer by the same rules, among the
//! importers whose manifests set `place` ([`named_for_place`], [`for_place`]).
//!
//! The importer is given the file's bytes, at most [`import::MAX_INPUT_BYTES`] of them: a file
//! named by `path` that only an importer can read is read no further than that. Its objects become
//! a new document whose artboard is their bounds (or, to place, the art of one group), and nothing
//! opens or is placed when the importer fails, declines, is cancelled or returns objects that don't
//! pass the checks a filter's output does.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::{Document, MAX_COORD, NodeId};
use vectorcraft_geom::Rect;
use vectorcraft_plugins::import::Mode;
use vectorcraft_plugins::{Manifest, Plugin, export, import, objects};

use super::super::{Result, bad, str_param};
use super::load::{Loaded, Source, detect, file_name};
use super::{Format, err, extension, file_stem, format};
use crate::EngineError;

const C: &str = "document.open";

/// The importer command `cmd`'s params `p` name (`importer`), if any; a value that isn't a string
/// is an error.
fn named<'a>(p: &'a Value, cmd: &str) -> Result<Option<&'a str>> {
    match p.get("importer").filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| bad(cmd, "`importer` must be a plug-in format id (see document.formats)")),
    }
}

/// Checks that importer `m` lists the extension of the file `name`: else command `cmd`'s error,
/// which names the extensions it lists.
fn check_extension(cmd: &str, m: &Manifest, name: &str) -> Result<()> {
    if m.extensions.contains(&extension(name)) {
        return Ok(());
    }
    let shown = vectorcraft_plugins::permissions::display_text(&m.name, vectorcraft_plugins::permissions::MAX_SHOWN_NAME);
    let exts: Vec<String> = m.extensions.iter().map(|e| format!(".{e}")).collect();
    Err(bad(cmd, format!("“{shown}” opens {} files, not `{}`", exts.join(", "), file_name(name))))
}

/// The importer that opens the file `name` holding `bytes` with `document.open`'s params `p`: the
/// one `importer` names, which must list the file's extension; else, when no built-in format reads
/// the file, the first installed importer that lists its extension; else `None`.
pub(super) fn pick(name: &str, bytes: &[u8], p: &Value) -> Result<Option<Arc<Plugin>>> {
    if let Some(id) = named(p, C)? {
        let plugin = import::named(id).map_err(|e| bad(C, e.to_string()))?;
        check_extension(C, plugin.manifest(), name)?;
        return Ok(Some(plugin));
    }
    if detect(name, bytes).is_some() {
        return Ok(None);
    }
    Ok(import::for_extension(&extension(name)).into_iter().next())
}

/// The importer `file.place`'s params `p` name (`importer`) for the file `name`, which must set
/// `place` in its manifest and list the file's extension (errors of command `cmd`); `None` when `p`
/// names none.
pub(crate) fn named_for_place(name: &str, p: &Value, cmd: &str) -> Result<Option<Arc<Plugin>>> {
    let Some(id) = named(p, cmd)? else { return Ok(None) };
    let plugin = import::named(id).map_err(|e| bad(cmd, e.to_string()))?;
    let m = plugin.manifest();
    if !m.place {
        return Err(match import::not_placing(m) {
            vectorcraft_plugins::Error::Params(why) => bad(cmd, why),
            e => bad(cmd, e.to_string()),
        });
    }
    check_extension(cmd, m, name)?;
    Ok(Some(plugin))
}

/// The importer File › Place uses for the file `name` when no built-in format reads it: the first
/// installed importer, by plug-in id, whose manifest sets `place` and that lists its extension.
pub(crate) fn for_place(name: &str) -> Option<Arc<Plugin>> {
    import::placing(&extension(name)).into_iter().next()
}

/// Whether an importer may read a file of extension `ext` for `mode` when none is named: no
/// built-in format reads the extension (for Place, as text, RTF or Word either) and an importer
/// that reads for `mode` lists it.
fn importer_reads(ext: &str, mode: Mode) -> bool {
    super::readable_format(ext).is_none()
        && match mode {
            Mode::Import => !import::for_extension(ext).is_empty(),
            Mode::Place => !super::PLACE_EXTS.contains(&ext) && !import::placing(ext).is_empty(),
        }
}

/// The error of the file `name`, past [`import::MAX_INPUT_BYTES`], which an importer would read
/// for `mode`.
fn too_large(name: &str, mode: Mode) -> EngineError {
    match mode {
        Mode::Import => err(format!("can't open `{}`: {}", file_name(name), import::too_large())),
        Mode::Place => failed(Mode::Place, name, import::too_large()),
    }
}

/// The error of the file `name`, which importer `e` couldn't read for `mode`.
fn failed(mode: Mode, name: &str, e: impl std::fmt::Display) -> EngineError {
    match mode {
        Mode::Import => err(format!("can't open `{}`: {e}", file_name(name))),
        Mode::Place => err(format!("can't place `{}`: {e}", file_name(name))),
    }
}

/// How much of a file [`detect`] is given: more than its content checks read.
const SNIFF_BYTES: usize = 64 << 10;

/// Refuses `document.open`'s `dataBase64` in `p` before it is decoded when it holds a file past
/// [`import::MAX_INPUT_BYTES`] that only an importer reads, by the rule of `read_path`.
pub(super) fn check_data(p: &Value) -> Result<()> {
    check_data_for(p, Mode::Import, C)
}

/// [`check_data`] for `mode` and command `cmd`: `file.place`'s params with [`Mode::Place`].
fn check_data_for(p: &Value, mode: Mode, cmd: &str) -> Result<()> {
    let Some(b64) = str_param(p, "dataBase64") else {
        return Ok(());
    };
    let name = str_param(p, "name").unwrap_or("Untitled");
    let explicit = named(p, cmd)?.is_some();
    if (!explicit && !importer_reads(&extension(name), mode)) || vectorcraft_format::base64_decoded_len(b64) <= import::MAX_INPUT_BYTES as u64 {
        return Ok(());
    }
    if !explicit {
        let start: Vec<u8> = b64.bytes().filter(|b| !b.is_ascii_whitespace()).take(SNIFF_BYTES / 3 * 4).collect();
        let head = std::str::from_utf8(&start).ok().and_then(vectorcraft_format::base64_decode);
        if head.is_some_and(|head| detect(name, &head).is_some()) {
            return Ok(());
        }
    }
    Err(too_large(name, mode))
}

/// The file at `path` for `document.open`'s params `p`. A file that only an importer reads (one
/// `importer` names, or an extension no built-in format reads that an importer lists) is read up
/// to [`import::MAX_INPUT_BYTES`]: past that, it is an error, unless the start of the file shows a
/// built-in format and no importer was named, which is read whole as any other file is. A file past
/// the limit that isn't read whole is reported from its size: with `importer` named, before any of
/// it is read, and otherwise after the start of it is read to look for a built-in format.
pub(super) fn read_path(path: &str, p: &Value) -> Result<Vec<u8>> {
    read_path_for(path, p, Mode::Import, C)
}

/// [`read_path`] for `mode` and command `cmd`: `file.place`'s params with [`Mode::Place`].
#[cfg(not(target_arch = "wasm32"))]
fn read_path_for(path: &str, p: &Value, mode: Mode, cmd: &str) -> Result<Vec<u8>> {
    let explicit = named(p, cmd)?.is_some();
    if explicit {
        return read_for_importer(path, mode);
    }
    if !importer_reads(&extension(path), mode) {
        return super::read_file(path);
    }
    let head = super::read_file_head(path, SNIFF_BYTES)?;
    if head.len() < SNIFF_BYTES {
        return Ok(head);
    }
    if detect(path, &head).is_some() {
        return super::read_file(path);
    }
    read_for_importer(path, mode)
}

/// The file at `path` for an importer: refused from its size, before any of it is read, when it is
/// past [`import::MAX_INPUT_BYTES`].
#[cfg(not(target_arch = "wasm32"))]
fn read_for_importer(path: &str, mode: Mode) -> Result<Vec<u8>> {
    super::may_read(path)?;
    let meta = std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    if meta.is_file() && meta.len() > import::MAX_INPUT_BYTES as u64 {
        return Err(too_large(path, mode));
    }
    // The file can grow after the size is read; anything but a regular file is refused here.
    let file = super::read_file_head(path, import::MAX_INPUT_BYTES + 1)?;
    if file.len() > import::MAX_INPUT_BYTES {
        return Err(too_large(path, mode));
    }
    Ok(file)
}

#[cfg(target_arch = "wasm32")]
fn read_path_for(path: &str, _: &Value, _: Mode, _: &str) -> Result<Vec<u8>> {
    super::read_file(path)
}

/// The file `file.place`'s params `p` name, as [`super::source`] reads it for command `cmd`, except
/// that a file an importer would place (one `importer` names, or an extension no built-in format
/// reads that an importer which places its files lists) is read, or decoded, only when it is at
/// most [`import::MAX_INPUT_BYTES`] long.
pub(crate) fn place_source<'a>(p: &'a Value, cmd: &str) -> Result<Source<'a>> {
    match str_param(p, "path") {
        Some(path) => Ok(Source { name: path, bytes: read_path_for(path, p, Mode::Place, cmd)?, path: Some(path) }),
        None => {
            check_data_for(p, Mode::Place, cmd)?;
            super::source(p, cmd)
        }
    }
}

/// The file `name` holding `bytes` read by importer `plugin` for `mode` with command `cmd`'s params
/// `p` (`params`: the importer's own parameters) → the importer's format, a new document holding its
/// objects on its first layer, their ids, and its warnings.
fn read(
    name: &str,
    bytes: &[u8],
    plugin: &Plugin,
    p: &Value,
    cmd: &str,
    mode: Mode,
) -> Result<(&'static Format, Document, Vec<NodeId>, Vec<String>)> {
    let m = plugin.manifest();
    let format = format(&export::format_id(&m.id)).filter(|f| f.read).ok_or_else(|| err(vectorcraft_plugins::Error::NotFound(m.id.clone())))?;
    let (ext, stem) = (extension(name), file_stem(name));
    let source = import::Source { extension: &ext, name: &stem };
    let params = p.get("params").cloned().unwrap_or(Value::Null);
    let imported = import::run_as(plugin, bytes, &params, source, mode).map_err(|e| match e {
        vectorcraft_plugins::Error::Cancelled => EngineError::Cancelled,
        vectorcraft_plugins::Error::Params(why) => bad(cmd, why),
        e => failed(mode, name, e),
    })?;
    let mut doc = Document::new(1.0, 1.0);
    let layer = doc.layers.first().map(|l| l.id);
    let admit = |fx: &[vectorcraft_doc::Effect], n: &vectorcraft_doc::Node| crate::cmd::plugin::admit_effects_for(cmd, fx, n);
    let ids = objects::apply_output(&mut doc, &[], imported.objects, layer, &admit).map_err(|e| failed(mode, name, e))?;
    Ok((format, doc, ids, imported.warnings))
}

/// The file `name` holding `bytes` read by importer `plugin` to place it, with `file.place`'s params
/// `p` (errors of command `cmd`) → the importer's format, the document holding its objects on its
/// first layer, and its warnings. Importer `plugin`'s manifest must set `place`.
pub(crate) fn read_for_place(name: &str, bytes: &[u8], plugin: &Plugin, p: &Value, cmd: &str) -> Result<(&'static Format, Document, Vec<String>)> {
    let (format, doc, _, warnings) = read(name, bytes, plugin, p, cmd, Mode::Place)?;
    Ok((format, doc, warnings))
}

/// The file `name` holding `bytes` read by importer `plugin` with `document.open`'s params `p`
/// (`params`: the importer's own parameters) into a new document, its artboard the bounds of what
/// the importer returned.
pub(super) fn load(name: &str, bytes: &[u8], plugin: &Plugin, p: &Value) -> Result<Loaded> {
    let (format, mut doc, ids, warnings) = read(name, bytes, plugin, p, C, Mode::Import)?;
    if let (Some(bounds), Some(board)) = (doc.bounds_of(&ids, true), doc.artboards.first_mut()) {
        board.rect = artboard_rect(bounds);
    }
    doc.title = file_name(name);
    Ok(Loaded { doc, format, warnings, restored: false, converted: false, preview_only: false, editing_data: false, native_pdf_compatible: None })
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
