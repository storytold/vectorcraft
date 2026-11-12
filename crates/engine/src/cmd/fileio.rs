//! Document file I/O for every frontend (desktop, web, CLI, control channel, headless MCP): open,
//! save, export and serialize by path or bytes (base64), the format table, `command.batch` and
//! `action.batch` (the Actions panel menu's Batch…).
//!
//! - `load`: `document.open` (native, legacy, SVG/SVGZ, PDF/.ai/.ait, DXF, HP-GL, raster images).
//! - `action_batch`: play an action on every file in a folder or every data set.
//! - `encode`: one encoder per writable format, with typed options parsed from the params.
//! - `svg`: the SVG Options (styling, fonts, images, object ids, artboards…).
//! - `export`: `document.export` / `serialize` / `exportSelection` / `exportForOffice`.
//! - `screens`: Export for Screens (`document.exportForScreens`, `document.exportSettings`); `zip`
//!   packs its files as one download, and reads the files of Word documents placed as type.
//! - `save`: `document.save`, Save As / a Copy / as Template, New from Template, Revert and the
//!   per-format options (`file.formatOptions`); [`save_with`] is the one save path of every frontend.
//! - `pdf`: PDF settings and presets for every PDF export, `document.exportPdf`.
//! - `pdfimport`: the PDF pages, box and password `document.open` and Place read, `document.pdfInfo`.
//! - `dxf`: DXF export options, `document.exportDxf`, and the formats that can't be written (DWG, PICT).
//! - `eps`: EPS export options (flattened transparency, previews, the embedded document), `document.exportEps`.
//! - `dxfimport`: the DXF options `document.open` and Place read, `document.dxfInfo`.
//! - `metafile`: EMF and WMF export, open and place.
//! - `hpgl`: HP-GL export options and plots.
//! - `importer`: files that importer plug-ins open (`document.open {importer}`, and extensions no
//!   built-in format reads).
//! - `mesh3d`: 3D assets as OBJ, glTF, USDA and USDZ mesh files (`assets.export3D`).
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
mod importer;
pub mod jxlread;
mod legacy_ai;
mod load;
mod mesh3d;
pub(crate) use load::{MAX_RASTER_ALLOC, size_note};
mod heifread;
mod hpgl;
mod jp2read;
mod metafile;
mod naming;
mod native;
mod pcxread;
pub mod pdf;
mod pdfimport;
pub mod pngtext;
pub mod ppi;
pub mod progress;
mod psdlayers;
pub mod psdread;
mod pxrread;
mod save;
mod screens;
mod svg;
mod text;
mod tgaread;
pub mod zip;

use serde_json::{Value, json};

pub use action_batch::batch_overwrites_one_file;
pub use dxf::{UNSUPPORTED, Unsupported, unsupported};
pub use encode::{ARTBOARD_PARAMS, ArtboardPick, Encoded, encode, encode_all, encode_with_warnings};
pub(crate) use encode::{anti_alias, background, with_single_artboard};
pub(crate) use export::with_artboard_fills;
pub(crate) use export::{export_crop, isolated};
pub use export::{export_doc, export_source};
use load::err;
pub(crate) use load::import_svg;
pub(crate) use load::is_placeholder;
pub(crate) use load::native_file;
pub(crate) use load::page_or_editing_data;
pub(crate) use load::source;
pub use load::{
    Loaded, RasterImage, ReadFile, detect, file_name, load, load_with, open_bytes, open_bytes_with, open_read, open_template, raster_image,
    read_for_open, texture_image,
};
pub(crate) use load::{check_files_not_lossy_overwrite, check_not_lossy_overwrite, lossy_source};
pub use load::{losses_summary, overwrite_losses};
pub use mesh3d::{DEFAULT_FORMAT_3D, FORMATS_3D, format_3d_label};
pub(crate) use mesh3d::{mesh_file_names, mesh_files};
pub use naming::{NameParts as FileNameParts, parse_date as parse_file_date, render as render_file_name};
pub(crate) use native::preview_png;
pub use native::with_compression_pref;
pub use pdfimport::{LoadOptions, page_document};
pub(crate) use save::job_for;
pub(crate) use save::save as save_document;
pub use save::{
    SAVE_FORMATS, SaveJob, SaveMode, SavePlan, StreamWrite, TEMPLATE_FORMATS, dialog_folder, documents_dialog_folder, export_folder,
    leave_out_unreadable_save_paths, reads_option as save_reads_option, remembered_save_options, save_filters, save_format, save_job, save_plan,
    save_with, stamp_save_dates, template_filters, templates_dialog_folder, templates_folder,
};
pub(crate) use screens::{
    PARAM_KEYS as SCREEN_PARAM_KEYS, ROW_KEYS as SCREEN_ROW_KEYS, UNWRITTEN_FORMATS as SCREEN_UNWRITTEN_FORMATS,
    check_settings as check_screen_settings, export as export_screens, safe_file_name, store_settings as store_screen_settings,
    with_export_preset as with_screen_export_preset,
};
pub use screens::{
    PRESET_FILE_EXTS as SCREEN_EXPORT_PRESET_EXTS, PRESET_FILE_FORMAT as SCREEN_EXPORT_PRESET_FORMAT, PRESET_KEYS as SCREEN_EXPORT_PRESET_KEYS,
    PRESETS as SCREEN_PRESETS, SHARED_KEYS as SCREEN_SETTINGS_KEYS, ScreenExport, ScreenExportPreset, ScreenSize,
    builtin_presets as screen_export_builtin_presets, first_file as screen_first_file, preset_list as screen_export_preset_list,
    preset_rows as screen_preset_rows,
};
pub use svg::options_map as svg_options;
/// Atomic file writes (a temporary file renamed over the target: a failed write never damages the
/// file it replaces), for the apps' own writers too.
#[cfg(not(target_arch = "wasm32"))]
pub use vectorcraft_format::write_atomic;

use super::{CommandSpec, Result, Session, always, bad, cmd, has_doc, has_selection};
use crate::EngineError;

mod access;
mod commands;
mod export_names;
mod formats;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use access::list_dir;
pub use access::{write_file_with, write_save_file, write_save_file_with};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests_readonly;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use access::{Placed, copy_new, file_metadata};
pub(crate) use access::{
    absolute_path, create_dir, document_path, file_created, file_stamp, read_file, read_file_head, shown_link, shown_path, write_file, write_new_file,
};
#[cfg(all(test, windows))]
pub(crate) use access::{document_path_on, document_path_with};
#[cfg(test)]
pub(crate) use access::{drive_share_of, share_of};
#[cfg(test)]
pub(crate) use access::{drive_share_path, share_path, touched};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use access::{may_read, may_write};
pub use commands::specs;
pub use export_names::artboard_file_names;
pub(crate) use export_names::{
    artboard_file_names_keeping_dots, default_name, merge, unique_file_names_keeping_dots, with_black_output, write_or_return,
};
use export_names::{distinct_names_by, write_encoded};
pub(crate) use formats::is_own_title;
use formats::{ARTBOARD, ARTBOARDS, RANGE, USE_ARTBOARDS, import_name, writable};
pub use formats::{
    DOC_EXTS, DOCX_EXTS, FORMATS, Format, FormatOption, OPEN_EXTS, PLACE_EXTS, RTF_EXTS, TEXT_EXTS, UNASSOCIATED_EXTS, builtin_open_filters,
    export_extensions, extension, file_stem, format, format_for_name, has_own_title, importer_formats, library_open_filters, open_extensions,
    open_filters, opens_as_other, opens_with_importer, place_filters, plugin_formats, read_extensions, readable_format, readable_format_for_name,
    titled_for_export, writable_format,
};

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_affinity;
#[cfg(test)]
mod tests_artboard_fill;
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
mod tests_selexport;
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
mod tests_hpgl;
#[cfg(test)]
mod tests_metafile;

#[cfg(test)]
mod tests_pdfoutput;
#[cfg(test)]
mod tests_tiffbmp;

#[cfg(test)]
mod tests_jxlread;
#[cfg(test)]
mod tests_pdfx;
#[cfg(test)]
mod tests_psd;
#[cfg(test)]
mod tests_psdlayers;
#[cfg(test)]
mod tests_psdread;

#[cfg(test)]
mod tests_heifread;
#[cfg(test)]
mod tests_jp2read;
#[cfg(test)]
mod tests_pcxread;
#[cfg(test)]
mod tests_pxrread;

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
