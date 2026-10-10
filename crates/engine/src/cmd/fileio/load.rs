//! `document.open`: every readable format into a new document.

use std::io::Cursor;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Affine;

use super::super::*;
use super::pdfimport::LoadOptions;
use super::{Format, SAVE_FORMATS, absolute_path, file_stamp, format, format_for_name, psdread, read_file};
use crate::EngineError;

/// A file read into a document, with its format and non-fatal import notes.
pub struct Loaded {
    pub doc: Document,
    pub format: &'static Format,
    pub warnings: Vec<String>,
    /// The document is the native one the file carries (SVG or PDF saved with Preserve Editing).
    pub restored: bool,
    /// An older native file (the former `.drawcraft` name or format version): saving it again
    /// rewrites it in today's format.
    pub converted: bool,
    /// Only a stand-in picture of the file (an Affinity document whose native data couldn't be
    /// read): Open shows it with its warning; Place, templates and libraries refuse it.
    pub preview_only: bool,
}

/// An image ready to embed: PNG/JPEG/GIF/WebP keep their bytes, other formats are stored as PNG
/// (what browsers, PDF and SVG viewers show).
pub struct RasterImage {
    /// Content key for `Document::images` (identical files share one blob).
    pub key: String,
    pub blob: ImageBlob,
    pub width: u32,
    pub height: u32,
    /// The resolution the file declares (see [`super::ppi::resolution`]).
    pub ppi: Option<(f64, f64)>,
}

pub(super) fn err(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

/// The last component of a path or file name.
pub fn file_name(name: &str) -> String {
    std::path::Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string())
}

/// The readable format of a file: from its content (magic bytes) when that tells, else from the
/// extension of `name`.
pub fn detect(name: &str, bytes: &[u8]) -> Option<&'static Format> {
    let by_name = format_for_name(name).filter(|f| f.read);
    if vectorcraft_affinity::is_affinity(bytes) {
        return format("affinity");
    }
    if vectorcraft_format::sniff(bytes) {
        // A template keeps its meaning from the extension, like .ait.
        return by_name.filter(|f| f.id == "template").or_else(|| format("vectorcraft"));
    }
    if vectorcraft_svg::is_svgz(bytes) {
        return format("svgz");
    }
    if vectorcraft_pdf::is_postscript(bytes) {
        // PostScript (.eps, and .ai saved without PDF compatibility) by any name; .ai and .ait
        // keep their meaning (a template), read as PostScript all the same.
        return by_name.filter(|f| matches!(f.id, "ai" | "ait")).or_else(|| format("eps"));
    }
    if bytes.starts_with(b"%PDF") {
        // .ai and .ait files are PDF inside; the extension keeps the template meaning.
        return by_name.filter(|f| matches!(f.id, "ai" | "ait")).or_else(|| format("pdf"));
    }
    if vectorcraft_cad::is_dxf(bytes) {
        return format("dxf");
    }
    if let Some(kind) = vectorcraft_metafile::sniff(bytes) {
        return format(kind.id());
    }
    if psdread::is_psd(bytes) {
        return format(if psdread::is_psb(bytes) { "psb" } else { "psd" });
    }
    if let Some(f) = image::guess_format(bytes).ok().and_then(image_format) {
        return Some(f);
    }
    if clipboard::looks_like_svg(&String::from_utf8_lossy(&bytes[..bytes.len().min(4096)])) {
        return format("svg");
    }
    by_name
}

/// The readable raster format behind an `image` crate format.
fn image_format(f: image::ImageFormat) -> Option<&'static Format> {
    f.extensions_str().iter().find_map(|e| format(e)).filter(|f| f.read && f.raster)
}

/// Why a file carrying editing data (an SVG or PDF saved with Preserve Editing) opened as plain
/// artwork: it was changed in another app after it was saved.
pub const EDITING_STALE: &str = "this file was changed in another app after it was saved with its editing data: it opened as plain artwork";
/// Why a file carrying editing data opened as plain artwork: the data can't be read.
pub const EDITING_DAMAGED: &str = "this file's editing data can't be read: it opened as plain artwork";

/// Preserve Editing, for every format that carries the native document: that document when the
/// file around it is unchanged (`editing`: whether it is, and how to get the document's bytes) and
/// it reads, else `import()`'s document with a first warning saying why the editing data wasn't
/// used → (document, warnings, restored).
fn restore_or_import(
    editing: Option<(bool, impl FnOnce() -> Option<Vec<u8>>)>,
    import: impl FnOnce() -> Result<(Document, Vec<String>)>,
) -> Result<(Document, Vec<String>, bool)> {
    let mut fallback = None;
    if let Some((intact, bytes)) = editing {
        let native = if intact { bytes() } else { None };
        match native.and_then(|b| native_file(&b).ok()) {
            Some((doc, _, warnings)) => return Ok((doc, warnings, true)),
            None => fallback = Some(if intact { EDITING_DAMAGED } else { EDITING_STALE }),
        }
    }
    let (doc, mut warnings) = import()?;
    if let Some(why) = fallback {
        warnings.insert(0, why.to_string());
    }
    Ok((doc, warnings, false))
}

/// A native file's document and details, the colour profiles it carries installed where missing
/// (with a warning for each that can't be used).
pub(crate) fn native_file(bytes: &[u8]) -> Result<(Document, vectorcraft_format::FileInfo, Vec<String>)> {
    let f = vectorcraft_format::load_file(bytes).map_err(err)?;
    let warnings = super::native::install_profiles(f.profiles);
    Ok((f.doc, f.info, warnings))
}

/// Read a file of any readable format (`name`: its file name or path, for the extension and title;
/// a path's folder is where an SVG's relative image links are found).
pub fn load(name: &str, bytes: &[u8]) -> Result<Loaded> {
    load_with(name, bytes, &LoadOptions::default())
}

/// A PDF-compatible `.ai` read through the editor's own copy of its art, which it carries: its
/// layers and hidden objects as they were, and the art outside its artboard (its PDF part has
/// only what is on the artboards), as an EPS of the editor is read. Anything else as it was.
fn through_editing_data(bytes: &[u8], opts: &LoadOptions, doc: Document, warnings: Vec<String>) -> (Document, Vec<String>) {
    if opts.pages.is_some() || !opts.layers || !opts.editing_data {
        return (doc, warnings);
    }
    let Some(private) = vectorcraft_pdf::illustrator_data(bytes, opts.password.as_deref()) else { return (doc, warnings) };
    let outlined = opts.text_as == vectorcraft_pdf::TextAs::Outlines;
    let (doc, mut warnings) = vectorcraft_eps::layered_ai(&private, doc, warnings, outlined);
    // The note that the art outside the artboard is lost is the reason the layers weren't read.
    if warnings.iter().any(|w| w.starts_with("the file's layers weren't read from its editing data")) {
        return (doc, warnings);
    }
    warnings.retain(|w| w != vectorcraft_pdf::OFF_ARTBOARD_NOTE);
    (doc, warnings)
}

/// A `.ai` saved without PDF compatibility (its PDF part is a placeholder page) from the editor's
/// own copy of its art alone: `None` when the file has none (or pages are picked, or layers or the
/// editing data are off), an error when it can't be read.
fn from_editing_data_alone(bytes: &[u8], opts: &LoadOptions) -> Option<Result<(Document, Vec<String>)>> {
    if opts.pages.is_some() || !opts.layers || !opts.editing_data {
        return None;
    }
    let private = vectorcraft_pdf::illustrator_data(bytes, opts.password.as_deref())?;
    Some(
        vectorcraft_eps::ai_alone(&private)
            .map_err(|why| err(format!("this file was saved without PDF compatibility, and its editing data can't be read ({why})"))),
    )
}

/// [`load`] with `document.open` options (the PDF pages, box and password; the DXF options).
pub fn load_with(name: &str, bytes: &[u8], opts: &LoadOptions) -> Result<Loaded> {
    let format = detect(name, bytes).ok_or_else(|| match super::unsupported(&super::extension(name)) {
        Some(u) => err(format!("can't open `{}`: {}", file_name(name), u.hint)),
        None => err(format!("can't open `{name}`: not a format VectorCraft reads (see document.formats)")),
    })?;
    let title = file_name(name);
    let mut converted = false;
    let mut preview_only = false;
    let (mut doc, warnings, restored) = match format.id {
        // EPS and PostScript .ai: the document our EPS files carry, else the PostScript read.
        _ if format.id == "eps" || vectorcraft_pdf::is_postscript(bytes) => {
            let editing = vectorcraft_eps::has_native(bytes).then_some((true, || vectorcraft_eps::native(bytes)));
            restore_or_import(editing, || {
                let r = vectorcraft_eps::import_with(bytes, opts.editing_data).map_err(|e| err(format!("can't open `{}`: {e}", file_name(name))))?;
                Ok((r.document, r.warnings))
            })?
        }
        "vectorcraft" | "template" => {
            let (d, info, warnings) = native_file(bytes)?;
            converted = info.is_old() || super::extension(name) == vectorcraft_format::LEGACY_EXTENSION;
            (d, warnings, false)
        }
        "svg" | "svgz" => {
            let text = vectorcraft_svg::text_of(bytes).map_err(err)?;
            let editing = vectorcraft_svg::editing(&text).map(|e| (e.intact, move || vectorcraft_format::base64_decode(&e.data)));
            // Relative links are found in the SVG's folder (when `name` is a path).
            let folder = std::path::Path::new(name).parent().map(|f| f.to_string_lossy()).filter(|f| !f.is_empty()).map(|f| absolute_path(&f));
            restore_or_import(editing, || import_svg(&text, folder.as_deref()))?
        }
        "pdf" | "ai" | "ait" => {
            // Saved with Preserve Editing: the document it carries, unless some pages are picked.
            let editing = opts
                .pages
                .is_none()
                .then(|| vectorcraft_pdf::editing_with(bytes, opts.password.as_deref()))
                .flatten()
                .map(|e| (e.intact, move || Some(e.data)));
            restore_or_import(editing, || match super::pdfimport::import(bytes, opts) {
                Ok((doc, warnings)) => Ok(through_editing_data(bytes, opts, doc, warnings)),
                // Saved without its PDF part: the editing copy is all the file has.
                Err(e) if e.to_string() == vectorcraft_pdf::PdfError::PlaceholderOnly.to_string() => {
                    from_editing_data_alone(bytes, opts).unwrap_or(Err(e))
                }
                Err(e) => Err(e),
            })?
        }
        "dxf" => {
            let (doc, warnings) = super::dxfimport::import(bytes, &opts.dxf)?;
            (doc, warnings, false)
        }
        "emf" | "wmf" => {
            let (doc, warnings) = super::metafile::import(bytes)?;
            (doc, warnings, false)
        }
        "affinity" => {
            let i = super::affinity::import(&title, bytes)?;
            preview_only = i.preview_only;
            (i.doc, i.warnings, false)
        }
        _ if format.raster => (raster_doc(&title, bytes)?, vec![], false),
        _ => return Err(err(format!("{} files can't be opened yet", format.label))),
    };
    // Threaded type read from a file flows through its frames.
    if !doc.text_threads.is_empty() && !restored {
        crate::cmd::threads::reflow(&Document::new(1.0, 1.0), &mut doc);
    }
    if let Some(mode) = opts.color_mode.filter(|m| *m != doc.color_mode) {
        super::super::colormgmt::set_color_mode(&mut doc, mode, true, None, opts.grays);
    }
    // Imports are named after the file; a native document keeps its own title (the tab shows the
    // file name once it has a path).
    if !matches!(format.id, "vectorcraft" | "template") || doc.title.is_empty() {
        doc.title = title;
    }
    Ok(Loaded { doc, format, warnings, restored, converted, preview_only })
}

/// Import SVG text, reading the files its images link to (relative links from `folder`, the SVG's
/// own): rasters stay linked, SVG files become art, missing ones placeholders (see
/// [`vectorcraft_svg::ImportOptions`]).
pub(crate) fn import_svg(text: &str, folder: Option<&str>) -> Result<(Document, Vec<String>)> {
    let read = |path: &str| {
        // Only files (not folders or devices) are read.
        file_stamp(path)?;
        let bytes = read_file(path).ok()?;
        let link = crate::cmd::links::link_info(path, &bytes);
        Some((bytes, link))
    };
    vectorcraft_svg::import_with(text, &vectorcraft_svg::ImportOptions { folder, read: Some(&read) }).map_err(err)
}

/// Open a file's bytes as the new active document (what `document.open` does) →
/// `{index, title, format, warnings, restored, missingLinks, modifiedLinks, updatedLinks}`. The
/// document keeps `path` (and its format, for Save) when Save can write that format back: not a
/// template, a partly read PDF, or a `.ai` file whose native document didn't come back. The
/// document's linked files are looked for from `path` ([`crate::cmd::links::resolve`]).
pub fn open_bytes(s: &mut Session, name: &str, bytes: &[u8], path: Option<String>) -> Result<Value> {
    open_bytes_with(s, name, bytes, path, &Value::Null)
}

/// [`open_bytes`] with the `document.open` options in `p` ([`LoadOptions::from_params`]).
pub fn open_bytes_with(s: &mut Session, name: &str, bytes: &[u8], path: Option<String>, p: &Value) -> Result<Value> {
    let opts = LoadOptions::from_params("document.open", p)?;
    open_loaded(s, load_with(name, bytes, &opts)?, path, &opts, false)
}

/// File → New from Template: any readable file (at `path`, if it is one: its links are looked for
/// from there) as a new untitled document.
pub fn open_template(s: &mut Session, name: &str, bytes: &[u8], path: Option<&str>) -> Result<Value> {
    let loaded = load(name, bytes)?;
    if loaded.preview_only {
        return Err(err(
            "only this Affinity file's embedded preview could be read: use File › Open to see it with its warning, or export SVG or PDF from Affinity first",
        ));
    }
    open_loaded(s, loaded, path.map(str::to_string), &LoadOptions::default(), true)
}

/// Make a loaded file the new active document; `opts` are the options it was read with.
fn open_loaded(s: &mut Session, loaded: Loaded, path: Option<String>, opts: &LoadOptions, as_template: bool) -> Result<Value> {
    let Loaded { mut doc, format, warnings, restored, converted, .. } = loaded;
    // What reading the file left out of the document, if anything.
    let losses: Vec<String> = warnings.iter().filter(|w| is_loss(w)).cloned().collect();
    let source_path = path.clone().filter(|_| !losses.is_empty());
    let links = crate::cmd::links::resolve(&mut doc, path.as_deref(), s.prefs.update_links == "automatically");
    // A template (saved by Save as Template, or an .ait/.vctemplate file) opens as a new untitled
    // document.
    let template = as_template || doc.template || matches!(format.id, "ait" | "template");
    if template {
        doc.template = false;
        doc.title = s.next_untitled();
    }
    // Save writes the file back only in a format it writes, only a whole PDF (see
    // [`LoadOptions::is_partial`]), and a .ai file only when it carried the native document (Save
    // writes .ai that way).
    let partial = format.id == "pdf" && opts.is_partial();
    let lossy_ai = format.id == "ai" && !restored;
    let path = path.filter(|_| !template && !partial && !lossy_ai && SAVE_FORMATS.contains(&format.id));
    let saves_back = path.is_some();
    let converted = saves_back && converted && s.prefs.append_converted;
    let index = s.add_document(doc, path);
    let st = s.doc_mut()?;
    if saves_back {
        st.format = format.id;
    }
    st.converted = converted;
    st.imported_from = source_path;
    st.import_losses = losses;
    let title = s.documents()[index].title();
    Ok(super::merge(json!({ "index": index, "title": title, "format": format.id, "warnings": warnings, "restored": restored }), links.to_json()))
}

/// A file named by a command's params: `{path}` (read from disk) or `{name, dataBase64}`.
pub(crate) struct Source<'a> {
    /// The path, or the given name (default "Untitled"): for the extension and the title.
    pub name: &'a str,
    pub bytes: Vec<u8>,
    pub path: Option<&'a str>,
}

/// The file `p` names for command `cmd` (see [`Source`]).
pub(crate) fn source<'a>(p: &'a Value, cmd: &str) -> Result<Source<'a>> {
    match (str_param(p, "path"), str_param(p, "dataBase64")) {
        (Some(path), _) => Ok(Source { name: path, bytes: read_file(path)?, path: Some(path) }),
        (None, Some(b64)) => {
            let bytes = vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad base64"))?;
            Ok(Source { name: str_param(p, "name").unwrap_or("Untitled"), bytes, path: None })
        }
        _ => Err(bad(cmd, "give path, or name and dataBase64")),
    }
}

pub(super) fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let src = source(p, "document.open")?;
    open_bytes_with(s, src.name, &src.bytes, src.path.map(str::to_string), p)
}

pub(super) fn new_from_template(s: &mut Session, p: &Value) -> Result<Value> {
    let src = source(p, "file.newFromTemplate")?;
    open_template(s, src.name, &src.bytes, src.path)
}

/// The most memory decoding one image may take: a 60 in print sheet at 300 ppi (18000 × 6600 px)
/// is ~475 MB as RGBA, past the decoder's 512 MB default once it is converted. The web app keeps
/// that default: wasm32 has 4 GB to address and aborts when an allocation fails.
const MAX_RASTER_ALLOC: u64 = if cfg!(target_arch = "wasm32") { 512 << 20 } else { 2 << 30 };

/// Decode an image's header (and, for formats stored as PNG, its pixels). CMYK TIFFs are kept as
/// they are, with their ink amounts ([`ImageBlob::cmyk`]); CMYK TIFFs with an alpha channel, which
/// the decoder can't read, become RGBA in the active colour settings' CMYK, as do CMYK Photoshop
/// documents, whose merged image is read ([`psdread`]).
pub fn raster_image(bytes: &[u8]) -> Result<RasterImage> {
    let ppi = super::ppi::resolution(bytes);
    if psdread::is_psd(bytes) {
        let cms = vectorcraft_color::cms::active();
        return as_png(psdread::decode(bytes, MAX_RASTER_ALLOC, |c| cms.cmyk_to_srgb(c, false)).map_err(err)?, ppi);
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(err)?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_RASTER_ALLOC);
    reader.limits(limits);
    let kind = reader.format().ok_or_else(|| err("not an image VectorCraft reads (see document.formats)"))?;
    let f = image_format(kind).ok_or_else(|| err(format!("{kind:?} images can't be opened (see document.formats)")))?;
    let cmyk = || ImageBlob::new(f.mime, bytes.to_vec()).cmyk().is_some();
    if matches!(f.id, "png" | "jpg" | "gif" | "webp") || (f.id == "tiff" && cmyk()) {
        let (width, height) = reader.into_dimensions().map_err(err)?;
        return stored(bytes.to_vec(), f.mime, (width, height), ppi);
    }
    let cmyka = || {
        let cms = vectorcraft_color::cms::active();
        vectorcraft_doc::cmyk::cmyka_tiff_rgba(bytes, |c| cms.cmyk_to_srgb(c, false))
    };
    let img = match (f.id == "tiff").then(cmyka).flatten() {
        Some(img) => img,
        None => reader.decode().map_err(err)?.to_rgba8(),
    };
    as_png(img, ppi)
}

/// `img` stored as a PNG that keeps the file's resolution.
fn as_png(img: image::RgbaImage, ppi: Option<(f64, f64)>) -> Result<RasterImage> {
    let size = img.dimensions();
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(err)?;
    let png = match ppi {
        Some(r) => super::ppi::with_png_resolution(&png, r),
        None => png,
    };
    stored(png, "image/png", size, ppi)
}

/// An image's encoded `bytes`, `width` × `height` pixels, ready to embed.
fn stored(bytes: Vec<u8>, mime: &'static str, (width, height): (u32, u32), ppi: Option<(f64, f64)>) -> Result<RasterImage> {
    if width == 0 || height == 0 {
        return Err(err("the image is empty"));
    }
    let blob = ImageBlob::new(mime, bytes);
    Ok(RasterImage { key: blob.content_key(), blob, width, height, ppi })
}

/// An image as a document of its physical size at the resolution it declares (as Place sizes it;
/// 72 ppi, 1 px = 1 pt, when it declares none), the image named after the file.
pub(super) fn raster_doc(name: &str, bytes: &[u8]) -> Result<Document> {
    let RasterImage { key, blob, width, height, ppi } = raster_image(bytes)?;
    let (sx, sy) = crate::cmd::place::pt_per_px(ppi);
    let mut d = Document::new(width as f64 * sx, height as f64 * sy);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let xf = Affine::scale_non_uniform(sx, sy);
    let mut n = Node::new(id, NodeKind::Image(ImageObject { key: key.clone(), width, height, xf, link: None, placement: Default::default() }));
    n.name = Some(name.to_string());
    d.images.insert(key, blob);
    d.insert(Some(layer), 0, n).map_err(err)?;
    Ok(d)
}

/// Does this import note say that the file has something the document doesn't?
fn is_loss(note: &str) -> bool {
    note == vectorcraft_pdf::OFF_ARTBOARD_NOTE || vectorcraft_eps::is_loss(note)
}

/// Writing the file `target` for document `st`: not over the file `st` was read from when reading
/// it left something out (hidden text, art or layers), unless `p` has `acknowledgeLoss: true`.
/// Another name keeps the original file, with all it has.
pub(crate) fn check_not_lossy_overwrite(st: &crate::DocState, target: &str, p: &Value, cmd: &str) -> Result<()> {
    let Some(losses) = overwrite_losses(st, target, p) else { return Ok(()) };
    let what = losses_summary(losses);
    Err(bad(
        cmd,
        format!(
            "{target} is the file this document was read from, and reading it left things out ({what}): writing over it would lose them for good. Write another file instead, or pass acknowledgeLoss: true to replace it anyway"
        ),
    ))
}

/// What writing the file `target` for document `st` would lose for good: the notes of what reading
/// it left out when `target` is the file `st` was read from, unless `p` has `acknowledgeLoss: true`.
/// The UI asks before such a write and repeats it with `acknowledgeLoss` ([`check_not_lossy_overwrite`]).
pub fn overwrite_losses<'a>(st: &'a crate::DocState, target: &str, p: &Value) -> Option<&'a [String]> {
    let source = st.imported_from.as_deref().filter(|_| !st.import_losses.is_empty())?;
    (!bool_or(p, "acknowledgeLoss", false) && same_file(source, target)).then_some(st.import_losses.as_slice())
}

/// Up to three of `losses`, quoted, and how many more.
pub fn losses_summary(losses: &[String]) -> String {
    let mut what = losses.iter().take(3).map(|n| format!("“{n}”")).collect::<Vec<_>>().join("; ");
    if losses.len() > 3 {
        what.push_str(&format!("; {} more", losses.len() - 3));
    }
    what
}

/// Do `a` and `b` name one file (spelled differently, or through a link)?
fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
