//! Document file I/O for every frontend (desktop, web, CLI, control channel, headless MCP): open,
//! save, export and serialize by path or bytes (base64), the format table, and `command.batch`.
//!
//! - `load`: `document.open` (native, legacy, SVG/SVGZ, PDF/.ai/.ait, raster images).
//! - `encode`: one encoder per writable format, with typed options parsed from the params.
//! - `export`: `document.export` / `serialize` / `exportSelection` / `exportForScreens`.
//! - `save`: `document.save`, `file.saveAsTemplate`.
//!
//! [`FORMATS`] is the single list of formats (append-only); open dialogs use [`open_filters`],
//! agents query `document.formats`.

mod batch;
mod encode;
mod export;
mod load;
mod save;

use serde_json::{Value, json};

pub use encode::{ARTBOARD_PARAMS, ArtboardPick, encode};
pub use load::{Loaded, RasterImage, detect, load, open_bytes, raster_image};

use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.open",
            "Open Document",
            [],
            None,
            "{path} or {name, dataBase64} → {index, title, format, warnings}; any readable format (see document.formats): .vectorcraft/.drawcraft, .svg/.svgz, .pdf/.ai, .ait, PNG/JPEG/GIF/WebP/TIFF/BMP (an image opens as a document of its pixel size). Templates (native templates, .ait) open as a new untitled document",
            always,
            load::open
        ),
        cmd!(
            "document.save",
            "Save Document",
            [],
            None,
            "{path?} native .vectorcraft (default: the document's path) → {path}; a never-saved document without path → {dataBase64} (stays modified)",
            has_doc,
            save::save
        ),
        cmd!(
            query "document.serialize",
            "Serialize Document",
            [],
            None,
            "{format?: vectorcraft (default)|svg|pdf|png|jpg|webp, …the format's options (see document.formats)} → {text} for svg, else {dataBase64}",
            has_doc,
            export::serialize
        ),
        cmd!(
            "document.export",
            "Export Document",
            [],
            None,
            "{path?, format?: svg|pdf|png|jpg|webp|vectorcraft (default: from the path's extension, else png), artboard?: 0, artboards?: [i…], range?: \"1-3, 5\" (1-based; PDF writes one page per artboard, default all; the other formats write one artboard), scale?: 1 (raster), quality?: 90 (jpg), outlineText?: bool (svg)} → {path, format, bytes}; no path → {dataBase64, format, bytes}. Never changes the document's path",
            has_doc,
            export::export
        ),
        cmd!(
            "document.exportSelection",
            "Export Selection…",
            ["File"],
            None,
            "{path?, format?: png|jpg|webp|svg|pdf (default: from the extension, else png), scale?: 1, …the format's options} the selected objects cropped to their bounds (template layers left out) → {path, bytes, bounds} (no path → {dataBase64, bounds})",
            has_selection,
            export::export_selection
        ),
        cmd!(
            "file.saveAsTemplate",
            "Save as Template…",
            ["File"],
            None,
            "{path?} a native copy that opens as a new untitled document (no path → {dataBase64})",
            has_doc,
            save::save_template
        ),
        cmd!(
            "document.exportForScreens",
            "Export for Screens",
            ["File", "Export"],
            None,
            "{folder?, artboards?: [index…] | range?: \"1-3\" (default all), formats?: [{format: png|jpg|webp|svg|pdf, scale?: 1 (raster only), suffix?: \"@2x\" (raster default: @{scale}x when scale ≠ 1; svg/pdf drop @Nx suffixes)}], prefix?} one file per artboard and format (a PDF holds its artboard alone; artboards with the same name, in any case, get -2, -3…; an unnamed one is Artboard-N) → {files: [path…]}; no folder → {files: [{name, dataBase64}]}",
            has_doc,
            export::export_for_screens
        ),
        cmd!(query "command.batch", "Batch", [], None, "{label?, commands: [{command, params}]} run several commands as ONE undo step; stops at the first error and rolls back", has_doc, batch::batch),
        cmd!(
            query "document.formats",
            "File Formats",
            [],
            None,
            "{} → {formats: [{id, label, extensions, mime, read, write, raster, options: {name: {type, default, description}}}], readable: [id…], writable: [id…], openExtensions: [ext…]}",
            always,
            formats
        ),
    ]
}

/// One option a format's encoder reads from the export params.
#[derive(Clone, Copy, Debug)]
pub struct FormatOption {
    pub name: &'static str,
    /// JSON type: `number`, `integer`, `boolean`, `string` or `array`.
    pub ty: &'static str,
    /// The default as a JSON literal (`"1"`, `"false"`, `"null"`).
    pub default: &'static str,
    pub description: &'static str,
}

/// One file format: how `document.open` recognises it and what `document.export` writes.
#[derive(Clone, Copy, Debug)]
pub struct Format {
    /// The `format` param value (`svg`, `png`, …).
    pub id: &'static str,
    /// Open/save dialog filter name.
    pub label: &'static str,
    /// Lower-case extensions without the dot; the first is the one exports use.
    pub extensions: &'static [&'static str],
    pub mime: &'static str,
    pub read: bool,
    pub write: bool,
    /// Pixels (export `scale` applies) rather than vectors.
    pub raster: bool,
    /// Options the encoder reads (writable formats).
    pub options: &'static [FormatOption],
}

impl Format {
    pub fn to_json(&self) -> Value {
        let options: serde_json::Map<String, Value> = self
            .options
            .iter()
            .map(|o| {
                let default: Value = serde_json::from_str(o.default).unwrap_or(Value::Null);
                (o.name.to_string(), json!({"type": o.ty, "default": default, "description": o.description}))
            })
            .collect();
        json!({
            "id": self.id,
            "label": self.label,
            "extensions": self.extensions,
            "mime": self.mime,
            "read": self.read,
            "write": self.write,
            "raster": self.raster,
            "options": options,
        })
    }
}

const ARTBOARD: FormatOption = FormatOption {
    name: "artboard",
    ty: "integer",
    default: "0",
    description: "0-based artboard to export (`artboards: [i]` or `range: \"n\"` naming one artboard work too)",
};
const ARTBOARDS: FormatOption =
    FormatOption { name: "artboards", ty: "array", default: "null", description: "0-based artboards, one page each (default: all)" };
const RANGE: FormatOption = FormatOption {
    name: "range",
    ty: "string",
    default: "null",
    description: "1-based artboards such as \"1-3, 5\" (wins over artboards and artboard)",
};
const SCALE: FormatOption = FormatOption { name: "scale", ty: "number", default: "1", description: "pixels per point (0.01–64)" };
const QUALITY: FormatOption = FormatOption { name: "quality", ty: "integer", default: "90", description: "JPEG quality 1–100" };
const OUTLINE_TEXT: FormatOption =
    FormatOption { name: "outlineText", ty: "boolean", default: "false", description: "text as glyph outlines (viewable without the fonts)" };

/// A format `document.open` reads but nothing writes yet.
const fn reader(id: &'static str, label: &'static str, extensions: &'static [&'static str], mime: &'static str, raster: bool) -> Format {
    Format { id, label, extensions, mime, read: true, write: false, raster, options: &[] }
}

/// Every format VectorCraft reads or writes. Append-only: new formats go at the end.
pub const FORMATS: &[Format] = &[
    Format {
        id: "vectorcraft",
        label: "VectorCraft",
        extensions: &[vectorcraft_format::EXTENSION, vectorcraft_format::LEGACY_EXTENSION],
        mime: "application/json",
        read: true,
        write: true,
        raster: false,
        options: &[],
    },
    Format {
        id: "svg",
        label: "SVG",
        extensions: &["svg"],
        mime: "image/svg+xml",
        read: true,
        write: true,
        raster: false,
        options: &[ARTBOARD, OUTLINE_TEXT],
    },
    reader("svgz", "SVG Compressed", &["svgz"], "image/svg+xml", false),
    Format {
        id: "pdf",
        label: "PDF",
        extensions: &["pdf"],
        mime: "application/pdf",
        read: true,
        write: true,
        raster: false,
        options: &[ARTBOARD, ARTBOARDS, RANGE],
    },
    reader("ai", "PDF-compatible .ai", &["ai"], "application/pdf", false),
    reader("ait", "PDF-compatible .ait template", &["ait"], "application/pdf", false),
    Format { id: "png", label: "PNG", extensions: &["png"], mime: "image/png", read: true, write: true, raster: true, options: &[ARTBOARD, SCALE] },
    Format {
        id: "jpg",
        label: "JPEG",
        extensions: &["jpg", "jpeg"],
        mime: "image/jpeg",
        read: true,
        write: true,
        raster: true,
        options: &[ARTBOARD, SCALE, QUALITY],
    },
    reader("gif", "GIF", &["gif"], "image/gif", true),
    Format {
        id: "webp",
        label: "WebP",
        extensions: &["webp"],
        mime: "image/webp",
        read: true,
        write: true,
        raster: true,
        options: &[ARTBOARD, SCALE],
    },
    reader("tiff", "TIFF", &["tif", "tiff"], "image/tiff", true),
    reader("bmp", "BMP", &["bmp"], "image/bmp", true),
];

/// Every extension `document.open` reads (the "All readable files" filter of open dialogs).
pub const OPEN_EXTS: &[&str] =
    &["vectorcraft", "drawcraft", "svg", "svgz", "pdf", "ai", "ait", "png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp"];

/// Open-dialog filters: "All readable files" first, then one per readable format.
pub fn open_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once(("All readable files", OPEN_EXTS)).chain(FORMATS.iter().filter(|f| f.read).map(|f| (f.label, f.extensions)))
}

/// A format by id or extension (any case, leading dot allowed; `jpeg` finds `jpg`).
pub fn format(id_or_ext: &str) -> Option<&'static Format> {
    let k = id_or_ext.trim_start_matches('.').to_ascii_lowercase();
    FORMATS.iter().find(|f| f.id == k).or_else(|| FORMATS.iter().find(|f| f.extensions.contains(&k.as_str())))
}

/// The lower-case extension of a file name or path (empty when it has none).
pub fn extension(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// The format a file name's extension names.
pub fn format_for_name(name: &str) -> Option<&'static Format> {
    Some(extension(name)).filter(|e| !e.is_empty()).and_then(|e| format(&e))
}

/// The format to write: `format` (an id or extension), else the path's extension, else PNG.
pub fn writable_format(format_param: Option<&str>, path: Option<&str>) -> std::result::Result<&'static Format, String> {
    let f = match (format_param, path.map(extension).filter(|e| !e.is_empty())) {
        (Some(f), _) => format(f).ok_or_else(|| format!("unknown format `{f}` (see document.formats)"))?,
        (None, Some(e)) => format(&e).ok_or_else(|| format!("unknown extension `.{e}`: pass `format` (see document.formats)"))?,
        (None, None) => format("png").ok_or("PNG export is not available")?,
    };
    if f.write { Ok(f) } else { Err(format!("{} files can be opened but not written (see document.formats)", f.label)) }
}

/// [`writable_format`] for command `cmd`.
fn writable(cmd: &str, format_param: Option<&str>, path: Option<&str>) -> Result<&'static Format> {
    writable_format(format_param, path).map_err(|e| bad(cmd, e))
}

fn formats(_: &mut Session, _: &Value) -> Result<Value> {
    let ids = |pick: fn(&Format) -> bool| FORMATS.iter().filter(|f| pick(f)).map(|f| f.id).collect::<Vec<_>>();
    Ok(json!({
        "formats": FORMATS.iter().map(Format::to_json).collect::<Vec<_>>(),
        "readable": ids(|f| f.read),
        "writable": ids(|f| f.write),
        "openExtensions": OPEN_EXTS,
    }))
}

// ---------- the file system (none on the web, where commands take and return bytes) ----------

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_file(path: &str, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

#[cfg(not(target_arch = "wasm32"))]
fn create_dir(path: &str) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

#[cfg(target_arch = "wasm32")]
fn no_fs(path: &str) -> EngineError {
    EngineError::Other(format!("{path}: no file system here (pass dataBase64 to open; omit the path to get dataBase64)"))
}

#[cfg(target_arch = "wasm32")]
fn read_file(path: &str) -> Result<Vec<u8>> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
fn write_file(path: &str, _: &[u8]) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
fn create_dir(path: &str) -> Result<()> {
    Err(no_fs(path))
}

/// Write `bytes` to `path` → `{path, bytes, …extra}`; with no path → `{dataBase64, bytes, …extra}`.
fn write_or_return(path: Option<&str>, bytes: &[u8], extra: Value) -> Result<Value> {
    let mut out = match path {
        Some(path) => {
            write_file(path, bytes)?;
            json!({ "path": path, "bytes": bytes.len() })
        }
        None => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes), "bytes": bytes.len() }),
    };
    if let (Some(o), Value::Object(e)) = (out.as_object_mut(), extra) {
        o.extend(e);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
