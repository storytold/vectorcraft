//! Document file I/O for every frontend (desktop, web, CLI, control channel, headless MCP): open,
//! save, export and serialize by path or bytes (base64), the format table, `command.batch` and
//! `action.batch` (the Actions panel menu's Batch…).
//!
//! - `load`: `document.open` (native, legacy, SVG/SVGZ, PDF/.ai/.ait, DXF, raster images).
//! - `action_batch`: play an action on every file in a folder or every data set.
//! - `encode`: one encoder per writable format, with typed options parsed from the params.
//! - `svg`: the SVG Options (styling, fonts, images, object ids, artboards…).
//! - `export`: `document.export` / `serialize` / `exportSelection` / `exportForOffice`.
//! - `screens`: Export for Screens (`document.exportForScreens`, `document.exportSettings`); `zip`
//!   packs its files as one download.
//! - `save`: `document.save`, Save As / a Copy / as Template, New from Template, Revert and the
//!   per-format options (`file.formatOptions`); [`save_with`] is the one save path of every frontend.
//! - `pdf`: PDF settings and presets for every PDF export, `document.exportPdf`.
//! - `pdfimport`: the PDF pages, box and password `document.open` and Place read, `document.pdfInfo`.
//! - `dxf`: DXF export options, `document.exportDxf`, and the formats that can't be written (DWG, PICT).
//! - `eps`: EPS export options (flattened transparency, previews, the embedded document), `document.exportEps`.
//! - `dxfimport`: the DXF options `document.open` and Place read, `document.dxfInfo`.
//! - `metafile`: EMF and WMF export, open and place.
//!
//! [`FORMATS`] is the single list of formats (append-only); open dialogs use [`open_filters`],
//! agents query `document.formats`.

mod action_batch;
mod affinity;
mod batch;
pub mod dxf;
pub mod dxfimport;
mod encode;
pub mod eps;
mod export;
pub(crate) mod imagemap;
mod legacy_ai;
mod load;
pub(crate) use load::{MAX_RASTER_ALLOC, too_large_note};
mod metafile;
mod naming;
mod native;
pub mod pdf;
mod pdfimport;
pub mod pngtext;
pub mod ppi;
pub mod psdread;
mod save;
mod screens;
mod svg;
mod text;
mod zip;

use serde_json::{Value, json};

pub use dxf::{UNSUPPORTED, Unsupported, unsupported};
pub use encode::{ARTBOARD_PARAMS, ArtboardPick, Encoded, encode, encode_all, encode_with_warnings};
pub(crate) use encode::{anti_alias, background, with_single_artboard};
pub use export::export_source;
pub(crate) use export::{export_crop, isolated};
pub(crate) use load::check_not_lossy_overwrite;
use load::err;
pub(crate) use load::import_svg;
pub(crate) use load::native_file;
pub(crate) use load::source;
pub use load::{
    Loaded, RasterImage, ReadFile, detect, file_name, load, load_with, open_bytes, open_bytes_with, open_read, open_template, raster_image,
    read_for_open, texture_image,
};
pub use load::{losses_summary, overwrite_losses};
pub use naming::{NameParts as FileNameParts, parse_date as parse_file_date, render as render_file_name};
pub(crate) use native::preview_png;
pub use native::with_compression_pref;
pub use pdfimport::{LoadOptions, page_document};
pub(crate) use save::job_for;
pub use save::{
    SAVE_FORMATS, SaveJob, SaveMode, SavePlan, StreamWrite, TEMPLATE_FORMATS, export_folder, save_filters, save_format, save_job, save_plan,
    save_with, stamp_save_dates, template_filters, templates_dialog_folder, templates_folder,
};
pub use screens::{
    PRESET_KEYS as SCREEN_EXPORT_PRESET_KEYS, PRESETS as SCREEN_PRESETS, ScreenExportPreset, ScreenSize,
    builtin_presets as screen_export_builtin_presets, preset_rows as screen_preset_rows,
};
pub(crate) use screens::{
    SHARED_KEYS as SCREEN_SETTINGS_KEYS, check_settings as check_screen_settings, export as export_screens, store_settings as store_screen_settings,
};
pub use svg::options_map as svg_options;
/// Atomic file writes (a temporary file renamed over the target: a failed write never damages the
/// file it replaces), for the apps' own writers too.
#[cfg(not(target_arch = "wasm32"))]
pub use vectorcraft_format::write_atomic;

use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.open",
            "Open Document",
            [],
            None,
            "{path | name | dataBase64 (a file's path, or its name with its bytes), pages?: \"2-3, 5\" (PDF/.ai: 1-based, default all; one artboard each) | page?: n, cropTo?: bounding|art|crop (default)|trim|bleed|media (the box each artboard gets; bounding: the art's bounds; trim: the bleed box round it becomes the document's bleed; a PDF-compatible .ai or a PDF saved with its editing data opens on its trim boxes for crop), password? (encrypted PDFs; see document.pdfInfo), textAs?: text (default: point type per line, lines wrapped in a frame as area type where they lay out the same, in the file's fonts by name; missing ones are listed in warnings) | outlines (glyph paths), layers?: true (default: optional content groups become layers with their visibility, print state and lock — art that is off comes in as a hidden layer; other art goes to a layer per page) | false (one layer per page, without the art that is off; Save then asks for a name), editingData?: true (default: an EPS, .ai or PDF that carries its editor's own copy of the art opens from it, with its layers, hidden objects, guides, artboards and the art outside them; a .ai or PDF whose type shows opens from its PDF part with textAs: outlines) | \"always\" (as true, and a .ai's editing copy is used even where it draws differently from its PDF part, which can be older; the warnings say so) | false (only what its page or PDF part prints, for print pipelines; a .ai saved without PDF compatibility then can't be opened), colorMode?: rgb|cmyk (the mode the document opens in, its colours converted as file.documentColorMode does, grays? as there; default: the file's — a PDF keeps CMYK, Gray and spot inks (spot swatches at a tint) and opens in CMYK when painted mostly in CMYK), grays?: as file.documentColorMode, dxf?: {layout?: \"Model\" (default) | a paper layout's name (document.dxfInfo lists them), unit?, scale? (the ratio: 1 unit of the art = scale drawing units; default: the drawing at 1:1 in its own unit), fit?: false (scale to fit an artboard of fitTo?: [612, 792], turned to the art's orientation), scaleLineweights?: false (lineweights scale with the art), center?: true (false: the drawing's origin on the artboard's bottom-left corner; fitted art's bottom-left corner), mergeLayers?: false (all art on one layer)} (DXF), crop?: as cropTo (its older name)} → {index, title, format, warnings, restored, missingLinks, modifiedLinks, updatedLinks: [{name, path, ids}]}; any readable format (see document.formats): .vectorcraft/.drawcraft, .svg/.svgz, .pdf/.ai, .ait, ASCII .dxf (each DXF layer with art a layer; blocks symbols; what can't come in is listed in warnings), PNG/JPEG/GIF/WebP/TIFF/BMP (an image opens as a document of its size at the resolution it declares, as file.place sizes it; 72 ppi, 1 px = 1 pt, when it declares none), .emf/.wmf (one artboard, the picture's frame; records VectorCraft doesn't read are skipped with one warning), .eps and PostScript .ai (one artboard, the bounding box: an EPS file VectorCraft wrote restores the document it carries, restored: true; other files are read by a PostScript interpreter — paths, colours and spot inks, clips, gradients, images, type in the fonts named — and come in as their TIFF preview, with a warning, when their PostScript can't be read). A PDF/.ai/.ait or SVG saved with Preserve Editing restores the native document it carries (restored: true; a PDF only when no pages are picked), unless the file was changed elsewhere since or the data can't be read: then its artwork is imported and the first warning says why. Templates (native templates, .ait) open as a new untitled document; a restored .ai keeps its path (Save writes .ai again). Linked images are read from their files (looked for at their path, then relative to the document): missing ones show their saved preview (links.relink), modified ones are read again only with Preferences › Update Links: Automatically (else links.update). An SVG's <image> files (relative links from the SVG's folder) stay linked, SVG files become art, missing ones a placeholder in their box (a warning and missingLinks)",
            always,
            load::open
        ),
        cmd!(
            query "file.list",
            "List Folder",
            [],
            None,
            "{path} the files and folders in folder `path`, by name: what an agent may open there (held to the automation read root when one is in force) → {path, entries: [{name, kind: file|folder|link, size? (files, bytes)}], truncated (more than 5000 entries)} (#1163)",
            always,
            file_list
        ),
        cmd!(
            query "document.serialize",
            "Serialize Document",
            [],
            None,
            "{format?: vectorcraft (default)|template|svg|svgz|pdf|ai|png|jpg|webp|gif|png8|txt|dxf|eps|emf|wmf|tiff|bmp|tga|psd, …the format's options (see document.formats; SVG ones also as svg: {…}), selectedOnly?: false (the selected objects alone, in their layers)} → {text, warnings} for svg (plus dataBase64, the file, when its encoding isn't UTF-8), else {dataBase64, warnings}; an SVG of several artboards also gives files: [{name, text}], linked images linked: [{name, dataBase64}]",
            has_doc,
            export::serialize
        ),
        cmd!(
            "document.export",
            "Export Document",
            [],
            None,
            "{path?, format?: svg|svgz|pdf|ai|png|jpg|webp|gif|png8|txt|dxf|eps|emf|wmf|tiff|bmp|tga|psd|vectorcraft|template (default: from the path's extension, else png; png8 writes an indexed .png), selectedOnly?: false (the selected objects alone, in their layers), artboard?: 0, artboards?: [i…], range?: \"1-3, 5\" | \"all\" (1-based; PDF and .ai write one page per artboard, default all; SVG writes one file per artboard, {stem}-{artboard}.svg; raster formats write one artboard), useArtboards?: true (raster: one file per chosen artboard, default all, {stem}-{artboard}.{ext}; pdf/ai: every page) | false (pdf/ai/raster: the bounds of the visible art; SVG has it as an SVG option), ppi?: 72 (raster formats: pixels per inch, stored in the file; wins over scale), scale?: 1 (pixels per point), background?: transparent|white|black|\"#rrggbb\" (jpg: white when transparent), antiAlias?: none|art (default)|type (text snapped to pixels), interlaced?: false (png, Adam7), quality?: 90 (jpg: 0–100), colorModel?: rgb|cmyk|gray (jpg), method?: baseline|optimized|progressive, scans?: 3 (3–5, progressive), embedIcc?: true, imageMap?: none|client|server (an HTML or NCSA map of the objects with a URL, written as <stem>.html / <stem>.map), colors?: 256 (gif/png8: 2–256), reduction?: perceptual|selective (default)|adaptive|web|blackWhite|gray, dither?: none|diffusion (default)|pattern|noise, ditherAmount?: 100, transparency?: true, matte?: white|\"#rrggbb\"|none, interlaced?, lossless?: true (webp: lossy WebP isn't available yet: written lossless, with a warning), encoding?: utf8|utf16 (txt: the stories in stacking order, back to front, a thread once; utf16 with a byte order mark), lineEndings?: lf|crlf, selectionOnly?: false, svg?: {styling, outlineText, images, objectIds, decimals, minify, responsive, useArtboards, preserveEditing, metadata, fewerTspans, hiddenLayers, encoding, profile, embedFonts} (see document.formats), …the SVG options given flat instead, …the PDF options of document.exportPdf, …the DXF options of document.exportDxf (useArtboards: one drawing per artboard), …the EPS options of document.exportEps (useArtboards: one file per artboard, {stem}_{artboard}.eps; else the visible art), …emf and wmf write one picture of the artboard (useArtboards: true one file per chosen artboard, false the bounds of the visible art; EMF keeps curves, clipping, transparent images and gradients as images clipped to their shape; WMF flattens curves into polygons behind a placeable header; what a format leaves out comes back in warnings), colorModel?: rgb|cmyk|gray (tiff: rgb keeps transparency as an alpha channel), lzw?: true, byteOrder?: little|big, embedIcc?: true, colorModel?: rgb|gray (bmp), depth?: 24 (bmp: 1 black and white, 4|8 a palette by reduction and dither, 16, 24, 32 with alpha), fileFormat?: windows|os2 (1, 4, 8 or 24 bits), rle?: false (4 and 8 bits, windows), flipRows?: false (top-down rows), depth?: 24 (tga: 16 one-bit alpha, 24, 32 with alpha), colorModel?: rgb|cmyk|gray (psd), layers?: true (each top-level layer a pixel layer with its opacity and blend mode; a background colour is a Background layer; false: one flat image, on white), maxEditability?: false (layers and sublayers become groups, each object a layer named as in the Layers panel, text by its text; clipping, masked or knockout layers stay one layer), hiddenLayers?: false (hidden layers and objects written hidden instead of left out), embedIcc?: true (at most 30000 pixels a side; formats without alpha are flattened on white)} → {path, format, bytes, warnings, files?: [path…] (several), linked?: [path…] (linked images, image maps)}; no path → {dataBase64, format, bytes, warnings, files?: [{name, dataBase64}], linked?: [{name, dataBase64}]}. Never changes the document's path",
            has_doc,
            export::export
        ),
        cmd!(
            "document.exportSelection",
            "Export Selection…",
            ["File"],
            None,
            "{path?, format?: png|jpg|webp|svg|svgz|pdf|ai|gif|png8|tiff|bmp|tga|psd|txt|dxf|eps|emf|wmf|vectorcraft|template (default: from the extension, else png), scale?: 1, …the format's options} the selected objects cropped to their bounds (template layers left out) → {path, format, bytes, warnings, bounds} (no path → {dataBase64, format, bytes, warnings, bounds}); warnings say what the format approximated or left out, as for document.export",
            has_selection,
            export::export_selection
        ),
        cmd!(
            "document.exportForScreens",
            "Export for Screens",
            ["File", "Export"],
            None,
            "{folder?, zip?: false (one store-only .zip of every file: no folder → {name, dataBase64, bytes, files: [name…]}; with a folder it is written there → {path, bytes, files}), artboards?: [index…] | range?: \"1-3\" (default all), assets?: [asset id…] (instead of artboards: each asset's art alone, cropped to its bounds and named after the asset; see assets.list), layers?: [layer id…] (instead of artboards: each layer's art alone, sublayers included, named after the layer), selection?: one|each (instead of artboards: the selected art as one file, or a file per selected object, named after it), fullDocument?: false (one file per format instead: a PDF of every artboard, other formats the bounds of all visible art, named after the document), includeBleed?: false (artboards grown by the document's bleed), exportPreset?: (a preset of document.exportPresets.list: its settings, under these params), subfolders?: false (each row's files in a sub-folder: its folder, else its size for raster (1x, 2x, 100w…) or its format (SVG, PDF)), folders?: none|rows (as subfolders)|format (a folder per extension: png, svg, pdf…)|item (a folder per artboard, asset, layer or object)|itemFormat (item/extension) (default: rows with subfolders or a preset, else none), name?: (a file name pattern instead of the item's name, with <filename> for the document, <name> for the artboard, asset, layer or object, <index> from 1, #### for the counter zero-padded to the run of #, <format>, <scale>, <suffix> (else the row's suffix ends the name), YYYYMMDD, YYYY-MM-DD, YYYY, <date>, <time>, <YYYY>, <MM>, <DD>, <hh>, <mm> and <ss>; a trailing .ext is dropped, as in `COMPANY-Logo-<filename>-####-YYYYMMDD`; different files it names alike get -2, -3…), counterStart?: 1, counterPer?: item (every file of an item has its number)|file, date?: YYYY-MM-DD (the date the pattern uses; default today, UTC), preset?: mobile (PNG 1x, 2x, 3x) | density (PNG 0.75x–4x in ldpi…xxxhdpi sub-folders) instead of formats (turns subfolders on), formats?: [{format: png|png8|jpg|webp|gif|tiff|bmp|tga|psd|svg|svgz|pdf|eps|ai|vectorcraft, scale?: 1 | \"2x\" | \"100w\" | \"100h\" | \"72ppi\", width?: px | height?: px | ppi? (raster only; width, then height, then ppi win over scale), suffix?: (raster default: @2x, @100w, @100h, none at 1x; vector formats drop size suffixes), folder?: (its sub-folder), quality?: (jpg 0–100), …the format's options (antiAlias, background, preset (pdf)…)}], settings?: {png|png8|jpg|webp|gif|tiff|bmp|tga|psd|svg|svgz|pdf|eps|ai|vectorcraft: {…options for every row of that format}} (rows' own options win), prefix?, antiAlias?: none|art|type (raster rows without their own), openLocation?: (remembered; the app shows the files after exporting)} one file per artboard and format (a PDF holds its artboard alone; artboards with the same name, in any case, get -2, -3…; an unnamed one is Artboard-N) → {files: [path…]}; no folder → {files: [{name, dataBase64}]} (names include sub-folders: 2x/Icon@2x.png). eps, ai and vectorcraft rows write a document of the item alone (its one artboard). The params (but zip and assets) are remembered in the document (document.exportSettings; not an undo step)",
            has_doc,
            screens::export_for_screens
        ),
        cmd!(
            query "document.exportPresets.list",
            "Export Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, settings}]} the built-in batch export presets (Logo Package, Print-Ready PDF), then the saved ones; any name works as document.exportForScreens' exportPreset",
            always,
            screens::presets_list
        ),
        cmd!(
            query "document.exportPresets.save",
            "Save Export Preset",
            [],
            None,
            "{name, settings: {formats, settings, preset, name, prefix, counterStart, counterPer, folders, subfolders, includeBleed, antiAlias, folder, fullDocument} (as document.exportForScreens; other keys are left out)} save (or replace) a batch export preset, kept with the preferences; built-in names are refused → {name, created, settings}",
            always,
            screens::presets_save
        ),
        cmd!(
            query "document.exportPresets.delete",
            "Delete Export Preset",
            [],
            None,
            "{name} delete a saved batch export preset (built-in ones stay) → {deleted}",
            always,
            screens::presets_delete
        ),
        cmd!(
            query "command.batch",
            "Batch",
            [],
            None,
            "{label?, commands: [{command, params}], artboard?: index (the active artboard; the app passes it: each edit.pasteInPlace, edit.pasteInFront, edit.pasteInBack, object.align, object.distribute, object.distributeSpacing or select.allOnArtboard step that names none gets it, and each artboard.fitToArt or artboard.fitToSelection step that names no index gets it as index, or the last artboard of a document with fewer)} run several commands as ONE undo step (one in each document they edit: steps may open, switch, close or revert documents); stops at the first error and rolls everything back, documents included",
            has_doc,
            batch::batch
        ),
        cmd!(
            "action.batch",
            "Batch…",
            [],
            None,
            "{commands: [{command, params}] (the action's steps), source?: folder (default) | datasets, folder? (required for folder: the source folder), includeSubdirs?: false, destination?: none (default: leave each file open) | saveAndClose (save over the source and close) | folder (save or export into destinationFolder and close), destinationFolder? (required when destination is folder; also the folder Override Save/Export write into), overrideOpen?: true (skip document.open / file.open steps: the batch opens each source file), overrideSave?: true (point document.save / file.save / file.saveAs at destinationFolder/{stem}.{ext}), overrideExport?: true (point document.export / document.exportForScreens / document.exportSelection the same way), stopOnError?: true, label?} play the commands on every readable file in folder (and its subfolders when includeSubdirs), or on every data set of the open document (source: datasets; File + Data Set Name for Override Save/Export). Caps: 500 files or data sets, 8 folder levels, 256 steps. Batches cannot nest. → {processed: [{path|dataset, result|error}…], count, failed, stopped}",
            always,
            action_batch::action_batch
        ),
        cmd!(
            query "document.formats",
            "File Formats",
            [],
            None,
            "{} → {formats: [{id, label, extensions, mime, read, write, raster, options: {name: {type, default, description}}}], readable: [id…], writable: [id…], openExtensions: [ext…], unsupported: [{id, label, extensions, hint}] (formats asked for that can be neither opened nor written, such as DWG and PICT, with what to use instead)}",
            always,
            formats
        ),
        cmd!(
            query "document.pdfInfo",
            "PDF Info",
            [],
            None,
            "{path | name? | dataBase64 (a file's path, or its bytes), password?, thumbnail?: page (1-based), thumbnailSize?: 160 (px, longest side), cropTo?: crop (the thumbnail's box)} → {pages, needsPassword, wrongPassword?, pageInfo: [{width, height (pt, as shown), rotation, boxes: {media, crop, bleed, trim, art: [x0, y0, x1, y1] (PDF space, pt)}}], thumbnail?: PNG dataBase64}; a page whose UserUnit (PDF 1.6) is above 1 gives its size and boxes in pt, its numbers times the UserUnit (one above 75,000 counts as 75,000); an encrypted PDF without its password → {pages: 0, needsPassword: true}",
            always,
            pdfimport::pdf_info
        ),
        cmd!(
            "document.exportForOffice",
            "Save for Office Documents…",
            ["File"],
            None,
            "{path?, ppi?: 150 (72, 150, 300… pixels per inch), transparent?: false (else on white), artboard?: 0, acknowledgeLoss?: bool (write over the file the document was read from although reading it left things out)} the artboard as a PNG for office documents and slides → {path, bytes, width, height} (pixels); no path → {dataBase64, bytes, width, height}",
            has_doc,
            export::export_for_office
        ),
        cmd!(
            query "document.exportSettings",
            "Export for Screens Settings",
            [],
            None,
            "{} → {settings}: the document.exportForScreens params the document last exported with ({} when never; saved with the document, the dialog reopens on them)",
            has_doc,
            screens::export_settings
        ),
    ]
    .into_iter()
    .chain(save::specs())
    .collect()
}

/// One option a format's encoder reads from the export params.
#[derive(Clone, Copy, Debug)]
pub struct FormatOption {
    pub name: &'static str,
    /// JSON type: `number`, `integer`, `boolean`, `string`, `array` or `object`.
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
const ARTBOARDS: FormatOption = FormatOption {
    name: "artboards",
    ty: "array",
    default: "null",
    description: "0-based artboards (PDF and AI: one page each, default all; SVG, and raster formats with useArtboards: one file each)",
};
const RANGE: FormatOption = FormatOption {
    name: "range",
    ty: "string",
    default: "null",
    description: "1-based artboards such as \"1-3, 5\", or \"all\" (wins over artboards and artboard)",
};
const SCALE: FormatOption = FormatOption { name: "scale", ty: "number", default: "1", description: "pixels per point (0.01–64)" };
const QUALITY: FormatOption =
    FormatOption { name: "quality", ty: "integer", default: "90", description: "JPEG quality 0–100 (the JPEG Options dialog shows 0–10)" };
const USE_ARTBOARDS: FormatOption = FormatOption {
    name: "useArtboards",
    ty: "boolean",
    default: "null",
    description: "true: every chosen artboard (default all), one file each named <file>-<artboard>.<ext> (PDF and AI: one page each); false: the bounds of the visible art",
};
const PPI: FormatOption = FormatOption {
    name: "ppi",
    ty: "number",
    default: "72",
    description: "resolution in pixels per inch (72 = one pixel per point; 150, 300…), stored in the file; wins over scale",
};
const BACKGROUND: FormatOption = FormatOption {
    name: "background",
    ty: "string",
    default: "\"transparent\"",
    description: "transparent, white, black or a colour such as \"#ff8800\" (JPEG: white when transparent)",
};
const ANTI_ALIAS: FormatOption = FormatOption {
    name: "antiAlias",
    ty: "string",
    default: "\"art\"",
    description: "none (hard pixel edges), art (smooth edges) or type (smooth, text snapped to whole pixels)",
};
const INTERLACED: FormatOption =
    FormatOption { name: "interlaced", ty: "boolean", default: "false", description: "Adam7 interlacing (the image builds up while it loads)" };
const COLOR_MODEL: FormatOption = FormatOption {
    name: "colorModel",
    ty: "string",
    default: "\"rgb\"",
    description: "rgb, cmyk (ink amounts in the working CMYK space: CMYK colours keep their inks) or gray",
};
const METHOD: FormatOption = FormatOption {
    name: "method",
    ty: "string",
    default: "\"baseline\"",
    description: "baseline (standard), optimized (smaller Huffman tables) or progressive (builds up in scans)",
};
const SCANS: FormatOption = FormatOption { name: "scans", ty: "integer", default: "3", description: "progressive scans, 3–5" };
const EMBED_ICC: FormatOption = FormatOption {
    name: "embedIcc",
    ty: "boolean",
    default: "true",
    description: "embed the colour profile: sRGB (RGB), the working CMYK space (CMYK) or gray with sRGB's tone curve",
};
const IMAGE_MAP: FormatOption = FormatOption {
    name: "imageMap",
    ty: "string",
    default: "\"none\"",
    description: "none, client (an HTML page with <map>, <stem>.html) or server (an NCSA <stem>.map): the areas of objects with a URL and an Image Map shape (attributes.set)",
};

const COLORS: FormatOption =
    FormatOption { name: "colors", ty: "integer", default: "256", description: "most palette entries, 2–256 (the transparent one included)" };
const REDUCTION: FormatOption = FormatOption {
    name: "reduction",
    ty: "string",
    default: "\"selective\"",
    description: "the palette: perceptual, selective (also keeps rarer colours, snaps near web colours), adaptive (most used), web (web-safe), blackWhite or gray; art with that many colours or fewer keeps them exactly",
};
const DITHER: FormatOption = FormatOption {
    name: "dither",
    ty: "string",
    default: "\"diffusion\"",
    description: "none, diffusion (Floyd–Steinberg), pattern (8×8 ordered) or noise",
};
const DITHER_AMOUNT: FormatOption = FormatOption { name: "ditherAmount", ty: "integer", default: "100", description: "dither strength 0–100" };
const TRANSPARENCY: FormatOption = FormatOption {
    name: "transparency",
    ty: "boolean",
    default: "true",
    description: "pixels under half opacity become one transparent entry (false: everything is blended over the matte)",
};
const MATTE: FormatOption = FormatOption {
    name: "matte",
    ty: "string",
    default: "\"white\"",
    description: "the colour partly transparent edges are blended over (\"#rrggbb\", white, black), or none (they keep their colour)",
};
const LOSSLESS: FormatOption = FormatOption {
    name: "lossless",
    ty: "boolean",
    default: "true",
    description: "lossless WebP; false asks for lossy, which isn't available yet (written lossless, with a warning)",
};
const WEBP_QUALITY: FormatOption =
    FormatOption { name: "quality", ty: "integer", default: "90", description: "lossy quality 0–100 (for when lossy WebP is available)" };
/// The options of the palette formats (PNG-8, GIF).
const PALETTE_OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    USE_ARTBOARDS,
    PPI,
    SCALE,
    BACKGROUND,
    ANTI_ALIAS,
    INTERLACED,
    COLORS,
    REDUCTION,
    DITHER,
    DITHER_AMOUNT,
    TRANSPARENCY,
    MATTE,
];

const LZW: FormatOption = FormatOption { name: "lzw", ty: "boolean", default: "true", description: "LZW compression (lossless, smaller files)" };
const BYTE_ORDER: FormatOption = FormatOption {
    name: "byteOrder",
    ty: "string",
    default: "\"little\"",
    description: "the order of the bytes in the file's numbers: little (II, as on PCs) or big (MM)",
};
const TIFF_OPTIONS: &[FormatOption] =
    &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, PPI, SCALE, BACKGROUND, ANTI_ALIAS, COLOR_MODEL, LZW, BYTE_ORDER, EMBED_ICC];
const BMP_COLOR_MODEL: FormatOption =
    FormatOption { name: "colorModel", ty: "string", default: "\"rgb\"", description: "rgb or gray (greys at every depth)" };
const BMP_DEPTH: FormatOption = FormatOption {
    name: "depth",
    ty: "integer",
    default: "24",
    description: "bits per pixel: 1 (black and white), 4 or 8 (a palette by reduction and dither), 16, 24, or 32 (keeps transparency); the others are flattened on white",
};
const FILE_FORMAT: FormatOption = FormatOption {
    name: "fileFormat",
    ty: "string",
    default: "\"windows\"",
    description: "windows, or os2 (1, 4, 8 or 24 bits, uncompressed, rows bottom-up)",
};
const RLE: FormatOption =
    FormatOption { name: "rle", ty: "boolean", default: "false", description: "run-length encode a 4- or 8-bit windows bitmap (RLE4, RLE8)" };
const FLIP_ROWS: FormatOption = FormatOption {
    name: "flipRows",
    ty: "boolean",
    default: "false",
    description: "rows top-down (a negative height) instead of bottom-up; not with rle or os2",
};
const BMP_OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    USE_ARTBOARDS,
    PPI,
    SCALE,
    BACKGROUND,
    ANTI_ALIAS,
    BMP_COLOR_MODEL,
    BMP_DEPTH,
    FILE_FORMAT,
    RLE,
    FLIP_ROWS,
    REDUCTION,
    DITHER,
    DITHER_AMOUNT,
];
const TGA_DEPTH: FormatOption = FormatOption {
    name: "depth",
    ty: "integer",
    default: "24",
    description: "bits per pixel: 16 (one-bit alpha), 24 (flattened on white) or 32 (keeps transparency)",
};
const TGA_OPTIONS: &[FormatOption] = &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, PPI, SCALE, BACKGROUND, ANTI_ALIAS, TGA_DEPTH];
const PSD_LAYERS: FormatOption = FormatOption {
    name: "layers",
    ty: "boolean",
    default: "true",
    description: "write layers: each top-level layer a pixel layer with its opacity and blend mode (a background colour is a Background layer); false: one flat image, on white where transparent",
};
const MAX_EDITABILITY: FormatOption = FormatOption {
    name: "maxEditability",
    ty: "boolean",
    default: "false",
    description: "with layers: layers and sublayers become groups and every object a layer of its own, named as the Layers panel names it (text by its text); clipping, masked or knockout layers stay one layer",
};
const PSD_HIDDEN_LAYERS: FormatOption = FormatOption {
    name: "hiddenLayers",
    ty: "boolean",
    default: "false",
    description: "with layers: hidden layers (and hidden objects) are written as hidden layers instead of left out",
};
const PSD_OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    USE_ARTBOARDS,
    PPI,
    SCALE,
    BACKGROUND,
    ANTI_ALIAS,
    COLOR_MODEL,
    PSD_LAYERS,
    MAX_EDITABILITY,
    PSD_HIDDEN_LAYERS,
    EMBED_ICC,
];

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
        options: native::OPTIONS,
    },
    Format { id: "svg", label: "SVG", extensions: &["svg"], mime: "image/svg+xml", read: true, write: true, raster: false, options: svg::OPTIONS },
    Format {
        id: "svgz",
        label: "SVG Compressed",
        extensions: &["svgz"],
        mime: "image/svg+xml",
        read: true,
        write: true,
        raster: false,
        options: svg::OPTIONS,
    },
    Format { id: "pdf", label: "PDF", extensions: &["pdf"], mime: "application/pdf", read: true, write: true, raster: false, options: pdf::OPTIONS },
    Format {
        id: "ai",
        label: "PDF-compatible .ai",
        extensions: &["ai"],
        mime: "application/pdf",
        read: true,
        // Save As writes it; exports write the same file.
        write: true,
        raster: false,
        options: native::AI_OPTIONS,
    },
    Format {
        id: "ait",
        label: "PDF-compatible .ait template",
        extensions: &["ait"],
        mime: "application/pdf",
        read: true,
        // Save as Template writes it, with the .ai file's options; exports don't.
        write: false,
        raster: false,
        options: native::AI_OPTIONS,
    },
    Format {
        id: "png",
        label: "PNG",
        extensions: &["png"],
        mime: "image/png",
        read: true,
        write: true,
        raster: true,
        options: &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, PPI, SCALE, BACKGROUND, ANTI_ALIAS, INTERLACED],
    },
    Format {
        id: "jpg",
        label: "JPEG",
        extensions: &["jpg", "jpeg"],
        mime: "image/jpeg",
        read: true,
        write: true,
        raster: true,
        options: &[
            ARTBOARD,
            ARTBOARDS,
            RANGE,
            USE_ARTBOARDS,
            PPI,
            SCALE,
            BACKGROUND,
            ANTI_ALIAS,
            QUALITY,
            COLOR_MODEL,
            METHOD,
            SCANS,
            EMBED_ICC,
            IMAGE_MAP,
        ],
    },
    Format { id: "gif", label: "GIF", extensions: &["gif"], mime: "image/gif", read: true, write: true, raster: true, options: PALETTE_OPTIONS },
    Format {
        id: "webp",
        label: "WebP",
        extensions: &["webp"],
        mime: "image/webp",
        read: true,
        write: true,
        raster: true,
        options: &[ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, PPI, SCALE, BACKGROUND, ANTI_ALIAS, LOSSLESS, WEBP_QUALITY],
    },
    Format {
        id: "tiff",
        label: "TIFF",
        extensions: &["tif", "tiff"],
        mime: "image/tiff",
        read: true,
        write: true,
        raster: true,
        options: TIFF_OPTIONS,
    },
    Format { id: "bmp", label: "BMP", extensions: &["bmp"], mime: "image/bmp", read: true, write: true, raster: true, options: BMP_OPTIONS },
    Format {
        id: "template",
        label: "VectorCraft Template",
        extensions: &["vctemplate"],
        mime: "application/json",
        read: true,
        write: true,
        raster: false,
        options: native::OPTIONS,
    },
    Format { id: "png8", label: "PNG-8", extensions: &["png"], mime: "image/png", read: false, write: true, raster: true, options: PALETTE_OPTIONS },
    Format { id: "txt", label: "Text", extensions: TEXT_EXTS, mime: "text/plain", read: false, write: true, raster: false, options: text::OPTIONS },
    Format { id: "dxf", label: "DXF", extensions: &["dxf"], mime: "image/vnd.dxf", read: true, write: true, raster: false, options: dxf::OPTIONS },
    Format {
        id: "eps",
        label: "EPS",
        extensions: &["eps"],
        mime: "application/postscript",
        read: true,
        write: true,
        raster: false,
        options: eps::OPTIONS,
    },
    Format { id: "emf", label: "EMF", extensions: &["emf"], mime: "image/emf", read: true, write: true, raster: false, options: metafile::OPTIONS },
    Format { id: "wmf", label: "WMF", extensions: &["wmf"], mime: "image/wmf", read: true, write: true, raster: false, options: metafile::OPTIONS },
    Format { id: "tga", label: "Targa", extensions: &["tga"], mime: "image/x-tga", read: false, write: true, raster: true, options: TGA_OPTIONS },
    Format { id: "psd", label: "PSD", extensions: &["psd"], mime: "image/x-psd", read: true, write: true, raster: true, options: PSD_OPTIONS },
    // Affinity Photo's `.afphoto` opens by its content but isn't listed: Finder shouldn't offer a
    // vector app for a raster editor's documents.
    reader("affinity", "Affinity", &["af", "afdesign", "afpub"], "application/vnd.affinity", false),
    // Photoshop's large document format: read like a PSD (its merged image), never written.
    reader("psb", "PSB", &["psb"], "image/x-psb", true),
];

/// Every extension `document.open` reads (the "All readable files" filter of open dialogs).
pub const OPEN_EXTS: &[&str] = &[
    "vectorcraft",
    "drawcraft",
    "svg",
    "svgz",
    "pdf",
    "ai",
    "ait",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "tif",
    "tiff",
    "bmp",
    "psd",
    "psb",
    "vctemplate",
    "dxf",
    "emf",
    "wmf",
    "eps",
    "af",
    "afdesign",
    "afpub",
];

/// Extensions in [`OPEN_EXTS`] the desktop packaging doesn't associate with VectorCraft: Photoshop
/// documents belong to raster editors, so Finder and file managers don't offer a vector app for
/// them (File › Open and Place still read them).
pub const UNASSOCIATED_EXTS: &[&str] = &["psd", "psb"];

/// The extension that picks each writable format when exporting (the format's first; PNG-8 shares
/// `.png` with PNG, so `.png` comes once), in [`FORMATS`] order.
pub fn export_extensions() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for e in FORMATS.iter().filter(|f| f.write).filter_map(|f| f.extensions.first()) {
        if !v.contains(e) {
            v.push(e);
        }
    }
    v
}

/// Text files: File → Place sets them as area type (Text Import Options).
pub const TEXT_EXTS: &[&str] = &["txt"];

/// Every extension File → Place reads: [`OPEN_EXTS`] and [`TEXT_EXTS`].
pub const PLACE_EXTS: &[&str] = &[
    "vectorcraft",
    "drawcraft",
    "svg",
    "svgz",
    "pdf",
    "ai",
    "ait",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "tif",
    "tiff",
    "bmp",
    "psd",
    "psb",
    "vctemplate",
    "dxf",
    "emf",
    "wmf",
    "eps",
    "af",
    "afdesign",
    "afpub",
    "txt",
];

/// One dialog filter per readable format.
fn format_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    FORMATS.iter().filter(|f| f.read).map(|f| (f.label, f.extensions))
}

/// Open-dialog filters: "All readable files" first, then one per readable format, then swatch
/// libraries (which open in the library panel), Libraries panel libraries, flattener, PDF and print
/// presets (imported), scripts (checked without running them) and plug-ins (installed).
pub fn open_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once(("All readable files", OPEN_EXTS))
        .chain(format_filters())
        .chain(std::iter::once(("Swatch libraries", super::swatchlib::LIBRARY_EXTS)))
        .chain(std::iter::once(("Libraries", super::library::LIBRARY_EXTS)))
        .chain(std::iter::once(("Flattener presets", super::flatten::PRESET_EXTS)))
        .chain(std::iter::once(("PDF presets", super::pdfcmds::PRESET_EXTS)))
        .chain(std::iter::once(("Print presets", super::printpresets::PRESET_EXTS)))
        .chain(std::iter::once(("Shortcut sets", super::shortcutset::EXTS)))
        .chain(std::iter::once(("Scripts", super::scripts::EXTS)))
        .chain(std::iter::once(("Plug-ins", super::plugin::EXTS)))
}

/// File → Place dialog filters: "All placeable files", then one per placeable format, then text.
pub fn place_filters() -> impl Iterator<Item = (&'static str, &'static [&'static str])> {
    std::iter::once(("All placeable files", PLACE_EXTS))
        .chain(format_filters().filter(|(_, exts)| exts.iter().all(|ext| PLACE_EXTS.contains(ext))))
        .chain(std::iter::once(("Text", TEXT_EXTS)))
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

/// A file name or path without its folder and extension (`/a/Poster.svg` → `Poster`).
pub fn file_stem(name: &str) -> String {
    std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string())
}

/// What a `format` export of `doc`, whose file is `path`, writes when it isn't `doc` itself: a
/// document that only has the Untitled-N name New gave it is titled after its file, as its tab is
/// (the title PDF, PNG and SVG metadata carry). A title of its own stays, and native files keep
/// the stored title.
pub fn titled_for_export(doc: &vectorcraft_doc::Document, path: Option<&str>, format: &str) -> Option<vectorcraft_doc::Document> {
    let untitled = doc.title.strip_prefix("Untitled-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let path = path.filter(|_| untitled && !matches!(format, "vectorcraft" | "template" | "ai"))?;
    let mut d = doc.clone();
    d.title = file_stem(path);
    Some(d)
}

/// The format a file name's extension names.
pub fn format_for_name(name: &str) -> Option<&'static Format> {
    Some(extension(name)).filter(|e| !e.is_empty()).and_then(|e| format(&e))
}

/// The format to write: `format` (an id or extension), else the path's extension, else PNG.
pub fn writable_format(format_param: Option<&str>, path: Option<&str>) -> std::result::Result<&'static Format, String> {
    // Formats that can't be written say what to use instead.
    let unknown = |what: String, key: &str| unsupported(key).map_or(what, |u| u.hint.to_string());
    let f = match (format_param, path.map(extension).filter(|e| !e.is_empty())) {
        (Some(f), _) => format(f).ok_or_else(|| unknown(format!("unknown format `{f}` (see document.formats)"), f))?,
        (None, Some(e)) => format(&e).ok_or_else(|| unknown(format!("unknown extension `.{e}`: pass `format` (see document.formats)"), &e))?,
        (None, None) => format("png").ok_or("no PNG encoder")?,
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
        "unsupported": UNSUPPORTED.iter().map(Unsupported::to_json).collect::<Vec<_>>(),
    }))
}

// ---------- the file system (none on the web, where commands take and return bytes) ----------
//
// Every path a command reads or writes goes through these, which check it against the automation
// roots in force ([`crate::file_access`]): a path outside them is an error (`file_stamp` and
// `file_created`: no file), as is any path once roots are in force without its kind of access.

/// `path` checked for reading against the automation roots in force.
#[cfg(not(target_arch = "wasm32"))]
fn may_read(path: &str) -> Result<()> {
    crate::file_access::check_read(path).map_err(EngineError::Other)
}

/// `path` checked for writing against the automation roots in force.
#[cfg(not(target_arch = "wasm32"))]
fn may_write(path: &str) -> Result<()> {
    crate::file_access::check_write(path).map_err(EngineError::Other)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>> {
    may_read(path)?;
    std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// The first `max` bytes of the file at `path` (all of it when it is shorter). Anything but a
/// regular file (a folder, a pipe, a device) is refused, also through a link.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_file_head(path: &str, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    may_read(path)?;
    if !std::fs::metadata(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?.is_file() {
        return Err(EngineError::Other(format!("{path}: not a file")));
    }
    let mut out = vec![];
    std::fs::File::open(path).and_then(|f| f.take(max as u64).read_to_end(&mut out)).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_file(path: &str, bytes: &[u8]) -> Result<()> {
    may_write(path)?;
    // Never a half-written file: see [`write_atomic`].
    write_atomic(std::path::Path::new(path), bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// [`write_file`] with what `fill` writes into the file (a save streamed into it).
#[cfg(not(target_arch = "wasm32"))]
pub fn write_file_with(path: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>) -> Result<()> {
    may_write(path)?;
    vectorcraft_format::write_atomic_with(std::path::Path::new(path), |f| fill(f)).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_dir(path: &str) -> Result<()> {
    may_write(path)?;
    std::fs::create_dir_all(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))
}

/// Write `bytes` to a new file at `path`. A file already at `path` is an error and is never
/// replaced. When the write fails part way, the file it started is removed.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_new_file(path: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    may_write(path)?;
    let err = |e: std::io::Error| EngineError::Other(format!("{path}: {e}"));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(err)?;
    let written = f.write_all(bytes).and_then(|()| vectorcraft_format::sync_file(&f));
    drop(f);
    written.map_err(|e| {
        // Best effort: the partly written file is all there is to clean up.
        let _ = std::fs::remove_file(path);
        err(e)
    })
}

/// The highest number [`copy_new`] gives a copy's name (`Name 99.png`).
#[cfg(not(target_arch = "wasm32"))]
const MAX_COPY_NUMBER: u32 = 99;

/// Where [`copy_new`] put a file.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) enum Placed {
    /// Copied to this new file.
    Copied(std::path::PathBuf),
    /// Kept as this file, which holds the same contents.
    Kept(std::path::PathBuf),
}

/// Put a copy of the file `from` into the folder `dir`, never replacing a file there. It tries
/// `Name.ext`, then `Name 2.ext` up to `Name 99.ext`, and stops at the first name that is free,
/// where `create` makes the copy, or that is a file holding the same contents (`same`, given that
/// file and its metadata), which is kept. When `create` fails with `AlreadyExists` (the name was
/// taken meanwhile), it tries the next name. `None` when no name is free or holds the same
/// contents; a name whose metadata can't be read counts as taken.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn copy_new(
    from: &std::path::Path,
    dir: &std::path::Path,
    create: impl Fn(&std::path::Path) -> std::io::Result<()>,
    same: impl Fn(&std::path::Path, &std::fs::Metadata) -> bool,
) -> std::io::Result<Option<Placed>> {
    use std::io::ErrorKind;
    let (Some(name), Some(stem)) = (from.file_name(), from.file_stem()) else { return Err(ErrorKind::InvalidInput.into()) };
    let ext = from.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 1..=MAX_COPY_NUMBER {
        let to = if n == 1 { dir.join(name) } else { dir.join(format!("{} {n}{ext}", stem.to_string_lossy())) };
        match std::fs::symlink_metadata(&to) {
            Err(e) if e.kind() == ErrorKind::NotFound => match create(&to) {
                Ok(()) => return Ok(Some(Placed::Copied(to))),
                // Taken meanwhile: the next name.
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            },
            Ok(m) if m.is_file() && same(&to, &m) => return Ok(Some(Placed::Kept(to))),
            _ => {}
        }
    }
    Ok(None)
}

/// The most entries `file.list` returns.
const LIST_MAX: usize = 5000;

fn file_list(_: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad("file.list", "missing `path`"))?;
    let (entries, truncated) = list_dir(path, LIST_MAX)?;
    Ok(json!({ "path": absolute_path(path), "entries": entries, "truncated": truncated }))
}

/// The entries of folder `path`, sorted by name: at most `max`, and whether there were more. A
/// link isn't followed: its target may lie outside the automation roots.
#[cfg(not(target_arch = "wasm32"))]
fn list_dir(path: &str, max: usize) -> Result<(Vec<Value>, bool)> {
    may_read(path)?;
    let dir = std::fs::read_dir(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    let mut entries: Vec<(String, Value)> = vec![];
    let mut truncated = false;
    for e in dir.flatten() {
        if entries.len() == max {
            truncated = true;
            break;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        let entry = if t.is_symlink() {
            json!({ "name": name, "kind": "link" })
        } else if t.is_dir() {
            json!({ "name": name, "kind": "folder" })
        } else {
            json!({ "name": name, "kind": "file", "size": e.metadata().map(|m| m.len()).ok() })
        };
        entries.push((name, entry));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((entries.into_iter().map(|(_, v)| v).collect(), truncated))
}

#[cfg(target_arch = "wasm32")]
fn list_dir(_: &str, _: usize) -> Result<(Vec<Value>, bool)> {
    Err(EngineError::Other("no file system here".into()))
}

/// A file's size (bytes) and modification time (ms since the Unix epoch, when the file system
/// keeps one); `None` when there is no file at `path` (or automation may not read it).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_stamp(path: &str) -> Option<(u64, Option<u64>)> {
    may_read(path).ok()?;
    let m = std::fs::metadata(path).ok().filter(std::fs::Metadata::is_file)?;
    let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64);
    Some((m.len(), modified))
}

/// When the file at `path` was created (ms since the Unix epoch), when the file system keeps it
/// (and automation may read it).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn file_created(path: &str) -> Option<u64> {
    may_read(path).ok()?;
    let t = std::fs::metadata(path).ok()?.created().ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn file_created(_: &str) -> Option<u64> {
    None
}

/// `path` made absolute against the working directory (as given when absolute already, or when
/// that fails).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn absolute_path(path: &str) -> String {
    if std::path::Path::new(path).is_absolute() {
        return path.to_string();
    }
    std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn file_stamp(_: &str) -> Option<(u64, Option<u64>)> {
    None
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn absolute_path(path: &str) -> String {
    path.to_string()
}

#[cfg(target_arch = "wasm32")]
fn no_fs(path: &str) -> EngineError {
    EngineError::Other(format!("{path}: no file system here (pass dataBase64 to open; omit the path to get dataBase64)"))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn read_file_head(path: &str, _: usize) -> Result<Vec<u8>> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_file(path: &str, _: &[u8]) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub fn write_file_with(path: &str, _: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn create_dir(path: &str) -> Result<()> {
    Err(no_fs(path))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write_new_file(path: &str, _: &[u8]) -> Result<()> {
    Err(no_fs(path))
}

/// File-name parts for artboards `boards` of `doc`: the artboard's name with unsafe characters as
/// `-`, `Artboard-N` when unnamed, and `-2`, `-3`… after a name already used (compared without
/// case: `Icon` and `icon` are one file on most desktop file systems).
pub fn artboard_file_names(doc: &vectorcraft_doc::Document, boards: &[usize]) -> Vec<String> {
    unique_file_names(boards.iter().map(|&b| (doc.artboards.get(b).map_or("", |a| a.name.as_str()), format!("Artboard-{}", b + 1))))
}

/// File-name parts for `(name, fallback)` pairs (artboards, assets): the name with unsafe
/// characters as `-`, the fallback when it is empty, and `-2`, `-3`… after a name already used
/// (compared without case).
pub(crate) fn unique_file_names<'a>(names: impl IntoIterator<Item = (&'a str, String)>) -> Vec<String> {
    let mut taken = std::collections::HashSet::new();
    names
        .into_iter()
        .map(|(name, fallback)| {
            let mut base: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
            if base.is_empty() {
                base = fallback;
            }
            let mut name = base.clone();
            for i in 2.. {
                if taken.insert(name.to_lowercase()) {
                    break;
                }
                name = format!("{base}-{i}");
            }
            name
        })
        .collect()
}

/// Write an export's files for the destination `path` → `{path, bytes, files?, linked?, …extra}`
/// (`files`: every file when there are several; `linked`: the images it links to); with no path
/// → the same with `dataBase64` and `{name, dataBase64}` rows, named after `name`.
fn write_encoded(path: Option<&str>, name: &str, doc: &vectorcraft_doc::Document, enc: &Encoded, extra: Value) -> Result<Value> {
    let files = enc.named(doc, path.unwrap_or(name));
    let (main, linked) = files.split_at(enc.files.len());
    let row = |(name, bytes): &(String, std::borrow::Cow<[u8]>)| match path {
        Some(_) => json!(name),
        None => json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) }),
    };
    if path.is_some() {
        for (p, bytes) in &files {
            write_file(p, bytes)?;
        }
    }
    let Some((first, bytes)) = main.first() else { return Err(EngineError::Other("the export wrote no file".into())) };
    let mut out = match path {
        Some(_) => json!({ "path": first, "bytes": bytes.len() }),
        None => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes), "bytes": bytes.len() }),
    };
    if main.len() > 1 {
        out["files"] = main.iter().map(row).collect();
    }
    if !linked.is_empty() {
        out["linked"] = linked.iter().map(row).collect();
    }
    Ok(merge(out, extra))
}

/// `a` with the fields of `b`.
pub(crate) fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(a), Value::Object(b)) = (a.as_object_mut(), b) {
        a.extend(b);
    }
    a
}

/// Appearance of Black › Printing / Exporting (#394): Output All Blacks as Rich Black makes RGB
/// raster exports draw pure CMYK black as RGB black (`richBlack` on the encoder params).
pub(crate) fn with_black_output(s: &Session, p: &Value) -> Value {
    if s.prefs.black_output == "rich" { merge(p.clone(), json!({ "richBlack": true })) } else { p.clone() }
}

/// A file name for bytes handed back without a path: the document's title with `ext`.
pub(crate) fn default_name(doc: &vectorcraft_doc::Document, ext: &str) -> String {
    let stem = std::path::Path::new(&doc.title).file_stem().map(|s| s.to_string_lossy().into_owned()).filter(|s| !s.is_empty());
    format!("{}.{ext}", stem.as_deref().unwrap_or("Untitled"))
}

/// Write `bytes` to `path` → `{path, bytes, …extra}`; with no path → `{dataBase64, bytes, …extra}`.
pub(crate) fn write_or_return(path: Option<&str>, bytes: &[u8], extra: Value) -> Result<Value> {
    let out = match path {
        Some(path) => {
            write_file(path, bytes)?;
            json!({ "path": path, "bytes": bytes.len() })
        }
        None => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes), "bytes": bytes.len() }),
    };
    Ok(merge(out, extra))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_affinity;
#[cfg(test)]
mod tests_pdf;
#[cfg(test)]
mod tests_pdfcolor;
#[cfg(test)]
mod tests_pdfedit;
#[cfg(test)]
mod tests_pdffidelity;
#[cfg(test)]
mod tests_pdfimport;
#[cfg(test)]
mod tests_svg;
#[cfg(test)]
mod tests_svgedit;

#[cfg(test)]
mod tests_raster;

#[cfg(test)]
mod tests_exportas;

#[cfg(test)]
mod tests_jpeg;

#[cfg(test)]
mod tests_palette;

#[cfg(test)]
mod tests_batch;
#[cfg(test)]
mod tests_screens;
#[cfg(test)]
mod tests_text;

#[cfg(test)]
mod tests_dxf;

#[cfg(test)]
mod tests_pdfsecurity;
#[cfg(test)]
mod tests_svgenc;
#[cfg(test)]
mod tests_svgimport;

#[cfg(test)]
mod tests_eps;

#[cfg(test)]
mod tests_dxfimport;
#[cfg(test)]
mod tests_metafile;

#[cfg(test)]
mod tests_pdfoutput;
#[cfg(test)]
mod tests_tiffbmp;

#[cfg(test)]
mod tests_pdfx;
#[cfg(test)]
mod tests_psd;
#[cfg(test)]
mod tests_psdread;

#[cfg(test)]
mod tests_epsimport;

#[cfg(test)]
mod tests_aiimport;

#[cfg(test)]
mod tests_pdflayers;

#[cfg(test)]
mod tests_pdfwebview;

#[cfg(test)]
mod tests_pdf13;
