//! File → Place: another file's art into the active document as one undo step that never touches
//! the clipboard.
//!
//! - A raster image becomes one image at 100% of its physical size: its pixels at the resolution
//!   the file declares ([`fileio::ppi`]; 72 ppi when it declares none), linked to its file with
//!   `link` ([`super::links`]).
//! - A PSD or PSB file placed with `layers` becomes one group of its layers as objects, embedded
//!   ([`fileio::psd_layers`]); otherwise it is an image like any other, its merged image. An
//!   embedded one brings its slices in as user slices, placed with its art (`importSlices`,
//!   [`fileio::psd_slices`]).
//! - An SVG becomes one group of its art; so does a DXF drawing, read with the DXF options
//!   (`dxf`, [`fileio::dxfimport`]), and an HP-GL plot file (`vectorcraft_cad::hpgl`).
//! - An EMF, WMF or PICT picture becomes one group, clipped to its frame (`crop: "crop"`) or
//!   bounded by its art (`crop: "bounding"`).
//! - An EPS file (or a PostScript .ai) becomes one group, clipped to its bounding box or bounded by
//!   its art: the document an EPS file of ours carries, else what its PostScript draws, else its
//!   preview ([`fileio::load`]). With Link, one placed from a file becomes a placed document
//!   instead, as a PDF page does.
//! - A DCS file, an EPS file with one program per printing plate, becomes one linked image of its
//!   plates ([`dcs`]); it is never embedded.
//! - A PDF/.ai/.ait page or a native document's artboard becomes one group, clipped to the page
//!   (`crop: "crop"`; a PDF page's `art`, `trim`, `bleed` or `media` box too) or bounded by its art
//!   (`crop: "bounding"`). With Link, a PDF/.ai/.ait page placed from a file becomes a placed
//!   document instead, as a VectorCraft document does (below).
//! - A text file becomes area type, decoded and cleaned up with the Text Import Options (`text`,
//!   [`text`]).
//! - An RTF file becomes area type with its character and paragraph formatting ([`rtf`]).
//! - A Word document (`.docx`) becomes area type with the character and paragraph formatting its
//!   styles and direct formatting give ([`docx`]).
//! - A Word 97-2003 document (`.doc`) becomes area type the same way, its main text read from its
//!   compound file, with a warning for each kind of content left out ([`doc`]).
//! - A VectorCraft document placed from a file with Link (the default) becomes a placed document:
//!   one locked object showing the artboard `page`, linked to the file and read again when it
//!   changes ([`document`]); without Link, an editable copy of its art.
//! - A file an importer plug-in whose manifest sets `place` reads (the one `importer` names, else
//!   the first by id that lists the extension of a file no built-in format reads) becomes one group
//!   of the objects it returned, embedded ([`fileio::importer`]).
//!
//! The art lands centred on `at`, fitted into `rect`, or (Replace) where the replaced object was,
//! with its transform; Template puts it on a new template layer. The images, symbols, patterns and
//! swatches it uses join the document ([`adopt`]). `file.place.queue` loads the place cursor (the
//! `place` tool) with several files, which emits one `file.place` per click or drag.

pub(crate) mod adopt;
pub(crate) mod dcs;
mod doc;
pub(crate) mod document;
pub(crate) mod docx;
mod rtf;
pub(super) mod text;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, Document, ImageObject, LinkInfo, Node, NodeId, NodeKind, PageBox, Scaling, SliceOptions, SymbolBoxes};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_pdf::CropTo;

use super::fileio::{self, Format, RasterImage};
use super::{
    CommandSpec, Result, Session, always, bad, bool_or, check_numbers, check_switches, cmd, finite_numbers, has_doc, object_param, str_param,
    switch_or, unique_name,
};
use crate::{EngineError, MAX_COORD};

const PLACE: &str = "file.place";
const QUEUE: &str = "file.place.queue";
/// The error of a file read apart from the session while another document is active, or after the
/// document it was read for closed ([`Session::place_read`], [`Session::place_queue_read`]).
const NOT_ACTIVE: &str = "the document it was prepared for is no longer active";
/// The most files one `file.place.queue` loads.
const MAX_QUEUE: usize = 100;
/// The longest thumbnail side `thumbnail` may ask for (px).
const MAX_THUMBNAIL: f64 = 512.0;
/// A text file's natural box (`file.place.info`, the place cursor): a letter page less 1 in margins.
const TEXT_FRAME: Rect = Rect::new(0.0, 0.0, 468.0, 648.0);
/// An encrypted PDF's box in the place cursor, which reads it without its password: a letter page.
const LOCKED_PAGE: Rect = Rect::new(0.0, 0.0, 612.0, 792.0);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.place",
            "Place…",
            ["File"],
            Some("Cmd+Shift+P"),
            "{path | name | dataBase64 (a file's path, or its name with its bytes), link?: true (a raster image keeps its file's path; a VectorCraft document at path becomes one locked placed document showing artboard `page`, linked to the file and read again when it changes (links.update; saving it in this app updates it at once), drawn as vectors in every output; so does a PDF/.ai page at path, showing its `crop` box, unless password or textAs: \"outlines\" is given, and an EPS file, a PostScript file or a PostScript .ai at path, showing the art false gives; false: an editable copy of its art, as from dataBase64; other files are embedded), text?: {characterSet?: \"unicode\" (UTF-8, or UTF-16 with a byte-order mark, or without one by dataset.import's rule: an even length, at least half of its two-byte units holding a NUL byte, the byte order that reads a line break, valid UTF-16 with no NUL character; other bytes as the platform's 8-bit set) | \"ansi\" (the platform's 8-bit set) | \"shiftJis\" | \"gb2312\" (read as GBK) | \"chineseBig5\" | \"cyrillic\" (windows-1251, x-mac-cyrillic on mac) | \"gb18030\" | \"greek\" (windows-1253) | \"turkish\" (windows-1254) | \"baltic\" (windows-1257) | \"centralEuropean\" (windows-1250) (the WHATWG Encoding Standard's decoders: a byte-order mark is read as ordinary bytes, a malformed sequence as U+FFFD), platform?: \"windows\" (ANSI is Windows-1252) | \"mac\" (ANSI is Mac Roman), removeLineReturns?: false (each block of lines becomes one paragraph; blank lines end paragraphs), removeParagraphReturns?: false (drop blank lines), replaceSpaces?: n (runs of n ≥ 2 spaces become a tab)} (a .txt file, placed as area type filling rect, the replaced object's bounds, or else the artboard less a 36 pt margin), template?: false (onto a new locked template layer below the current layer), replace?: false (swap the one selected object, keeping its stacking place and transform; no at/rect; refused while an object of a symbol instance's art is selected), at?: [x, y] center (default: the first artboard's center; transformed art must stay within ±4,000,000 pt on either axis, including template placement), rect?: [x, y, width, height] fit inside; aspect kept (wins over at), page?: 1 (PDF/.ai page, or a native document's artboard), crop?: \"crop\" (clipped to that page or artboard, default; an SVG, only when given: framed and clipped by its viewBox, else by its art's bounds) | \"bounding\" (the bounds of the art that shows, with its strokes and effects, as an export measures them: hidden objects and guides left out) | \"art\" | \"trim\" | \"bleed\" | \"media\" (a PDF page's boxes), password? (an encrypted PDF), layers?: false (a PSD or PSB file: true embeds its layers as one group named after the file, read as document.open reads them: an image for each layer and a group for each layer group, with the layer's name, visibility, opacity, blend mode and opacity mask, at the place its merged image would take and clipped to the canvas when layers, hidden ones included, reach past it; never linked: link is then false unless given, and link: true is refused; a file without layers is embedded as its merged image, and so are one whose layers are all left out and one whose layers can't be read, with a warning), hiddenLayers?: false (with layers: true, the hidden layers come in hidden instead of being left out; refused without it), importSlices?: bool (a PSD or PSB file: its slices become user slices, moved and scaled with the placed art, in the same undo step; default true when the file is embedded, false when it is linked, and true with link is refused; slices the file generated automatically are left out), dxf?: {…the DXF options of document.open} (a .dxf drawing: fit fills the artboard under at, else the first; fitted or uncentred art lands with its drawing's artboard on that artboard, else it is centred on at), showImportOptions?: true (in the app, a text file without text, a DXF drawing without dxf, a PSD or PSB file embedded without layers (with two layers or more, or with slices) and a PDF with several pages or a password and no page ask for those options first, and an encrypted PDF given page and no password asks for its password; false places them with the options last confirmed, a PDF on page 1 (or page), and asks only an encrypted PDF's password), importer?: \"plugin.<id>\" (an importer plug-in whose manifest sets place, see document.formats: it reads the file, which must have an extension it lists, also one a built-in format reads; without importer, a file no built-in format reads, by its content or its extension, is read by the first such importer by id that lists its extension; it is given the file, at most 128 MiB, with _context.mode \"place\", and its visible layers' objects (all objects without layers), guides left out, are placed as one group named after the file, from the bottom layer upward in each layer's listed order (returned object order without layers), embedded whatever link says), params?: {…} (the importer's own parameters, see plugin.info)} → {ids, name, format, linked, width, height, warnings, image?: the id of the image of a DCS file or PSD/PSB file placed in a clip group, which the Links commands take, slices?: [the ids of the user slices a PSD file brought in]}. A PSD or PSB file that names a clipping path places in a clip group of that path (even-odd), as does its merged image in document.open. Raster images come in at 100% of their physical size (the file's ppi, else 72); SVG, DXF, HP-GL (.plt, .hpgl, .hgl, .hpg), PDF/.ai, EMF/WMF and PICT (.pct, .pic, .pict; clipped to the picture's frame), EPS (clipped to its bounding box; as document.open reads it), a .ai saved without PDF compatibility (from its editing data, as document.open reads it: the art of the layers that show on artboard `page`, clipped to that artboard unless crop is \"bounding\") and native documents as one group, with the images, symbols, patterns, swatches and brushes they use. A DCS file (an EPS file with one program per printing plate, DCS 1.0 or 2.0) is placed only from its path with link (else an error, as are document.open and links.embed): one linked image of its plates mixed into CMYK, clipped by the clipping path its composite is drawn through, its spot plates added as spot swatches in the process equivalents the file gives (a swatch of the same name is kept, with a warning); print separations and Separations Preview show each plate on its own ink, and plate files of their own are read from its folder. One undo step; selects what it placed (unless on a template layer); never touches the clipboard. A PNG, JPEG, GIF or WebP image whose pixels can't be read (a file cut short) is an error, and nothing is placed",
            has_doc,
            place
        ),
        cmd!(
            query "file.place.info",
            "Placed File Info",
            [],
            None,
            "{path | name | dataBase64 (a file's path, or its name with its bytes), page?, crop?, password?, thumbnail?: px, link?: bool, dxf?: {…}, layers?, hiddenLayers?, importSlices? (all as file.place)} what file.place would place, without placing it → {name, format, width, height (pt at 100%), pixelWidth?, pixelHeight?, ppi?: [x, y], colorMode?: RGB|Grayscale|CMYK (raster images; opaque CMYK PSD/PSB merged images keep 8- or 16-bit inks within VectorCraft's 67,108,864-pixel native CMYK limit and allocation allowance; other CMYK composites use the RGB decoder's allocation limit), plates?: [ink names] (a DCS file), slices?: the user slices a PSD or PSB file would bring in, warnings, thumbnailBase64?: PNG of at most `thumbnail` (≤ 512) px on its longer side}",
            always,
            info
        ),
        cmd!(
            query "image.info",
            "Image Info",
            [],
            None,
            "{id?} an image object (default: the one selected) → {id, name, linked, link (the linked file's path; null when embedded, or when automation read roots are in force and the file is outside them), colorMode: RGB|Grayscale|CMYK, pixelWidth, pixelHeight, ppi: [x, y] at its placed size, width, height}",
            has_doc,
            image_info
        ),
        cmd!(
            query "file.place.queue",
            "Load Place Cursor",
            [],
            None,
            "{paths?: [path…], files?: [{name, dataBase64}…], link?, template?, page?, crop?, text?, dxf?, layers?, hiddenLayers?, importSlices?, thumbnail?: px} load the place cursor (the `place` tool) with up to 100 files (as file.place reads them): a click places the current file at 100% with its top-left corner there, a drag places it at the dragged size (aspect kept), ←/→ and ↑/↓ cycle the files, Esc discards the current one; each placement is a file.place, and the previous tool returns after the last; an encrypted PDF, .ai or .ait file stays in the cursor at 612 × 792 pt (VectorCraft's own size) with needsPassword: true and no thumbnail, and a click on it in the app asks for its password (headless, its file.place fails for want of the password) → {count, files: [{name, format, width, height, thumbnailBase64?, needsPassword?}], skipped: [{name, error}]}",
            has_doc,
            queue
        ),
    ]
}

// ---------- reading the file ----------

/// A file's art at its natural size, before it joins a document.
enum Art {
    /// An image, with the clipping path its file names, in its pixel space (a PSD or PSB file's,
    /// [`fileio::psdread::clipping_path`]).
    Image(RasterImage, Option<vectorcraft_geom::PathData>),
    /// A text file's, an RTF file's or a Word document's text, placed as area type filling the
    /// natural box.
    Text(text::Story),
    /// A VectorCraft document, a PDF/.ai page or an EPS file placed linked: the file read, the
    /// link, and whether the art's bounds (not the artboard or page) are shown.
    Document(document::Source, LinkInfo, bool),
    /// A DCS file: one linked image of its plates, clipped by its clipping path ([`dcs`]).
    Dcs(Box<dcs::DcsArt>),
    Vector {
        /// The loaded document: the source of the art's resources.
        src: Box<Document>,
        nodes: Vec<Node>,
        /// Crop: the page or artboard the art is clipped to.
        clip: Option<Rect>,
    },
}

struct Loaded {
    /// The file name (the placed object's name).
    name: String,
    format: &'static Format,
    art: Art,
    /// The art's natural box (what `at` centres and `rect` fits).
    natural: Rect,
    warnings: Vec<String>,
    /// The link to a raster image's file or a placed document's (for Link), when read from a path.
    link: Option<LinkInfo>,
    /// The artboard a DXF drawing fitted to an artboard, or not centred, was laid out on: it lands
    /// on the target artboard the same way.
    board: Option<Rect>,
}

/// A PSD or PSB file's slices, embedded with `importSlices`: their rectangles over the natural box
/// of its art, and their options.
type Slices = Vec<(Rect, SliceOptions)>;

/// The most bytes [`head_is_dcs`] reads from the start of a file.
pub const DCS_HEAD_BYTES: usize = dcs::HEAD_BYTES;

/// Does `head`, the start of a file `size` bytes long (at most [`DCS_HEAD_BYTES`] of it, or all of
/// it), name DCS plates? `file.place` places such a file as a linked image, never as a placed
/// document. `None` when `head` can't tell.
pub fn head_is_dcs(head: &[u8], size: u64) -> Option<bool> {
    dcs::head_is_dcs(head, size)
}

/// Points per pixel of a raster image at the resolution it declares (72 ppi when none).
pub(crate) fn pt_per_px(ppi: Option<(f64, f64)>) -> (f64, f64) {
    let (x, y) = ppi.unwrap_or((72.0, 72.0));
    (72.0 / x, 72.0 / y)
}

/// [`load_sliced`] without the slices.
fn load(p: &Value, cmd: &str, board: Option<Rect>, link_pdf: bool) -> Result<Loaded> {
    load_into(p, cmd, board, link_pdf, &mut vec![])
}

/// [`load_into`], with the slices of a PSD or PSB file that `importSlices` brings in.
fn load_sliced(p: &Value, cmd: &str, board: Option<Rect>, link_pdf: bool) -> Result<(Loaded, Slices)> {
    let mut slices = vec![];
    let l = load_into(p, cmd, board, link_pdf, &mut slices)?;
    Ok((l, slices))
}

/// The file `p` names, read into placeable art; a DXF drawing's Fit to Artboard fills `board`
/// (unless the params name their own box). A PDF/.ai page or an EPS file read from a path is placed
/// linked with Link when `link_pdf` (false for the queries, which read it embedded). A PSD or PSB
/// file's slices go to `slices`. A file an importer plug-in places (the one `importer` names, or one
/// no built-in format reads) is read by it ([`plugin_art`]).
fn load_into(p: &Value, cmd: &str, board: Option<Rect>, link_pdf: bool, slices_out: &mut Slices) -> Result<Loaded> {
    let src = fileio::importer::place_source(p, cmd)?;
    let name = fileio::file_name(src.name);
    if let Some(plugin) = fileio::importer::named_for_place(src.name, p, cmd)? {
        return plugin_art(name, src.name, &src.bytes, &plugin, p, cmd);
    }
    // An Affinity file is recognised by its signature, whatever its name says.
    if fileio::TEXT_EXTS.contains(&fileio::extension(src.name).as_str()) && !vectorcraft_affinity::is_affinity(&src.bytes) {
        let text = text::import(&src.bytes, text::TextOptions::parse(p, cmd)?, cmd)?;
        if text.trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        let art = Art::Text(text::Story::plain(text));
        return Ok(Loaded { name, format: &text::FORMAT, art, natural: TEXT_FRAME, warnings: vec![], link: None, board: None });
    }
    // An RTF file is recognized by its content, whatever its name says (a .txt file is plain text).
    if rtf::is_rtf(&src.bytes) {
        let story = rtf::import(&src.bytes, cmd)?;
        if story.plain_text().trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(Loaded { name, format: &rtf::FORMAT, art: Art::Text(story), natural: TEXT_FRAME, warnings: vec![], link: None, board: None });
    }
    // A Word document is recognized by its name or by its package's `word/document.xml`.
    if docx::is_docx(src.name, &src.bytes) {
        let story = docx::import(&src.bytes, cmd)?;
        if story.plain_text().trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(Loaded { name, format: &docx::FORMAT, art: Art::Text(story), natural: TEXT_FRAME, warnings: vec![], link: None, board: None });
    }
    // A Word 97-2003 document is recognized by its name or by its compound file's WordDocument
    // stream.
    if doc::is_doc(src.name, &src.bytes) {
        let (story, warnings) = doc::import(&src.bytes, cmd)?;
        if story.plain_text().trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(Loaded { name, format: &doc::FORMAT, art: Art::Text(story), natural: TEXT_FRAME, warnings, link: None, board: None });
    }
    let Some(format) = fileio::detect(src.name, &src.bytes) else {
        return match fileio::importer::for_place(src.name) {
            Some(plugin) => plugin_art(name, src.name, &src.bytes, &plugin, p, cmd),
            None => Err(bad(cmd, format!("can't place `{name}`: not a format VectorCraft reads (see document.formats)"))),
        };
    };
    let page = match p.get("page") {
        None => 1,
        Some(v) => v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))? as usize,
    };
    let mut opts = fileio::LoadOptions::from_params(cmd, p)?;
    if let Some(b) = board
        && p.get("dxf").and_then(|d| d.get("fitTo")).is_none()
    {
        opts.dxf.fit_to = (b.width(), b.height());
    }
    // PostScript .ai files are read like EPS.
    let pdf = matches!(format.id, "pdf" | "ai" | "ait") && !vectorcraft_pdf::is_postscript(&src.bytes);
    if !pdf && !matches!(opts.crop, CropTo::Crop | CropTo::Bounding) {
        return Err(bad(
            cmd,
            format!("crop `{}`: only PDF pages have art, trim, bleed and media boxes; use \"crop\" or \"bounding\"", opts.crop.id()),
        ));
    }
    let crop = opts.crop != CropTo::Bounding;
    // A DCS file stays linked to its file, which its plates print from; its parameters are checked
    // as an EPS file's are.
    if dcs::is_dcs(&src.bytes) {
        let path = src.path.filter(|_| bool_or(p, "link", true)).map(fileio::absolute_path).ok_or_else(|| bad(cmd, dcs::LINK_ONLY))?;
        let a = dcs::read(&src.bytes, Some(&path)).map_err(|e| bad(cmd, format!("{name}: {e}")))?;
        let link = super::links::link_info(&path, &src.bytes);
        let (natural, warnings) = (a.page, a.warnings.clone());
        return Ok(Loaded { name, format, art: Art::Dcs(Box::new(a)), natural, warnings, link: Some(link), board: None });
    }
    // A VectorCraft document: placed linked from its file, else as an editable copy.
    if matches!(format.id, "vectorcraft" | "template") {
        let page = page as u32;
        if let Some(path) = src.path.filter(|_| bool_or(p, "link", true)) {
            let s = document::source(src.name, &src.bytes, Some(path), page, !crop, PageBox::Crop, cmd)?;
            let link = LinkInfo { page: Some(page), ..super::links::link_info(path, &src.bytes) };
            let (natural, warnings) = (s.frame, s.warnings.clone());
            return Ok(Loaded { name, format, art: Art::Document(s, link.clone(), !crop), natural, warnings, link: Some(link), board: None });
        }
        let a = document::document_art(src.name, &src.bytes, src.path, page, !crop, PageBox::Crop, cmd)?;
        let art = Art::Vector { src: Box::new(a.doc), nodes: a.nodes, clip: a.clip };
        return Ok(Loaded { name, format, art, natural: a.frame, warnings: a.warnings, link: None, board: None });
    }
    // A PSD or PSB file placed with `layers`: its layers as objects, never linked. One without
    // layers, or whose layers can't be read, is embedded as its merged image.
    let (mut embed, mut unread) = (false, vec![]);
    let mut slices_from = None;
    if matches!(format.id, "psd" | "psb") {
        let layers = switch_or(p, "layers", false, cmd)?;
        if switch_or(p, "hiddenLayers", false, cmd)? && !layers {
            return Err(bad(cmd, "hiddenLayers applies to a PSD file placed with layers: true"));
        }
        // Slices come in with an embedded file (default on); a linked file shows its flat image.
        let linked = !layers && src.path.is_some() && bool_or(p, "link", true);
        let slices = switch_or(p, "importSlices", !linked, cmd)?;
        if slices && linked {
            return Err(bad(cmd, "importSlices: a linked PSD file is placed as its flat image; give link: false to embed it with its slices"));
        }
        if slices {
            slices_from = Some(&src.bytes[..]);
        }
        if layers {
            // Layers are embedded: a link shows the file's flat image.
            if switch_or(p, "link", false, cmd)? {
                return Err(bad(cmd, "layers: a linked PSD file is placed as its flat image; leave link out or give link: false"));
            }
            embed = true;
            match fileio::psd_layers(&src.bytes, opts.hidden_layers) {
                Ok(Some((doc, mut warnings))) => match psd_groups(doc) {
                    Some((art, natural)) => {
                        *slices_out = psd_slices(slices_from, natural, &mut warnings);
                        return Ok(Loaded { name, format, art, natural, warnings, link: None, board: None });
                    }
                    // No layer has art: the merged image, embedded.
                    None => unread = warnings,
                },
                // No layers: the merged image, embedded.
                Ok(None) => {}
                Err(e) => unread.push(format!("the layers can't be read ({e}): the file was placed as its merged image")),
            }
        }
    }
    if format.raster {
        let img = fileio::raster_image(&src.bytes)?;
        let (sx, sy) = pt_per_px(img.ppi);
        let natural = Rect::new(0.0, 0.0, img.width as f64 * sx, img.height as f64 * sy);
        let link = src.path.filter(|_| !embed).map(|path| super::links::link_info(path, &src.bytes));
        let mut warnings: Vec<String> = unread.into_iter().chain(fileio::size_note(&img.blob.bytes)).collect();
        *slices_out = psd_slices(slices_from, natural, &mut warnings);
        let clip = img.clip.clone().map(|(path, _)| path);
        return Ok(Loaded { name, format, art: Art::Image(img, clip), natural, warnings, link, board: None });
    }
    if pdf {
        // Linked: placed like a VectorCraft document, read again with the page's box only. A page
        // asked for with a password or with its text as outlines is embedded.
        let linked = link_pdf && bool_or(p, "link", true) && opts.password.is_none() && opts.text_as == vectorcraft_pdf::TextAs::Text;
        if let Some(path) = src.path.filter(|_| linked) {
            let page = page as u32;
            let s = document::source(src.name, &src.bytes, Some(path), page, !crop, document::page_box(opts.crop), cmd)?;
            let link = LinkInfo { page: Some(page), ..super::links::link_info(path, &src.bytes) };
            let (natural, warnings) = (s.frame, s.warnings.clone());
            return Ok(Loaded { name, format, art: Art::Document(s, link.clone(), !crop), natural, warnings, link: Some(link), board: None });
        }
        let a = pdf_art(src.name, (&src.bytes, None), src.path, page, opts, !crop, cmd)?;
        let art = Art::Vector { src: Box::new(a.doc), nodes: a.nodes, clip: a.clip };
        return Ok(Loaded { name, format, art, natural: a.frame, warnings: a.warnings, link: None, board: None });
    }
    // An EPS file (or a PostScript .ai) with Link: placed like a PDF page, its art read as below.
    if let Some(path) = src.path.filter(|_| link_pdf && bool_or(p, "link", true) && document::is_postscript(&src.bytes)) {
        let page = page as u32;
        let s = document::source(src.name, &src.bytes, Some(path), page, !crop, PageBox::Crop, cmd)?;
        let link = LinkInfo { page: Some(page), ..super::links::link_info(path, &src.bytes) };
        let (natural, warnings) = (s.frame, s.warnings.clone());
        return Ok(Loaded { name, format, art: Art::Document(s, link.clone(), !crop), natural, warnings, link: Some(link), board: None });
    }
    let anchored = format.id == "dxf" && (opts.dxf.fit || !opts.dxf.center);
    let (mut doc, warnings) = {
        // Of the open options, only a DXF drawing's apply (the colour mode stays the file's).
        let o = fileio::LoadOptions { dxf: opts.dxf, ..Default::default() };
        let mut l = fileio::load_with(src.name, &src.bytes, &o)?;
        // An Affinity file whose native document couldn't be read is only its preview: it
        // must never silently stand in for the art.
        if l.preview_only {
            return Err(bad(
                cmd,
                format!(
                    "{name}: only this Affinity file's embedded preview could be read; use File › Open to see it with its warning, or export SVG or PDF from Affinity before placing"
                ),
            ));
        }
        // Linked images show their files, found from its folder.
        super::links::resolve(&mut l.doc, super::links::LinkBase::file(src.path), false, false);
        (l.doc, l.warnings)
    };
    doc.drop_edit_modes();
    let board = doc.artboards.first().map(|a| a.rect).filter(|_| anchored);
    let layers = std::mem::take(&mut doc.layers);
    let symbols = doc.symbol_boxes();
    let (nodes, clip) = if matches!(format.id, "svg" | "svgz" | "dxf" | "hpgl") {
        // An SVG with `crop: "crop"` asked for is framed and clipped by its viewBox (its artboard),
        // as a PDF by its page: art drawn on a shared canvas keeps its place and scale (#788). By
        // default, as Illustrator places one, it is its art's bounds.
        let framed = matches!(format.id, "svg" | "svgz") && crop && p.get("crop").is_some();
        let frame = doc.artboards.first().map(|a| a.rect).filter(|_| framed);
        (art_of(layer_art(&layers)), frame)
    } else {
        let n = doc.artboards.len();
        let pages = if n == 1 { "page" } else { "pages" };
        let board = doc.artboards.get(page - 1).map(|a| a.rect).ok_or_else(|| bad(cmd, format!("page {page}: `{name}` has {n} {pages}")))?;
        // An EPS file's art is whatever lies on the artboard.
        (on_artboard(&symbols, &layers, board), crop.then_some(board))
    };
    let natural = clip.or_else(|| shown_bounds(&symbols, &nodes));
    let natural = natural.filter(|r| r.width() > 0.0 || r.height() > 0.0).ok_or_else(|| bad(cmd, format!("`{name}` has no art to place")))?;
    Ok(Loaded { name, format, art: Art::Vector { src: Box::new(doc), nodes, clip }, natural, warnings, link: None, board })
}

/// The file `src` (named `name`) holding `bytes`, read by importer plug-in `plugin` with `p`'s
/// `params`, as art to place: its objects in paint order, guides left out, in a group whose natural
/// box is the bounds of what they draw. It is embedded: an importer reads the file once, so there is
/// nothing to link to.
fn plugin_art(name: String, src: &str, bytes: &[u8], plugin: &vectorcraft_plugins::Plugin, p: &Value, cmd: &str) -> Result<Loaded> {
    let (format, mut doc, warnings) = fileio::importer::read_for_place(src, bytes, plugin, p, cmd)?;
    let layers = std::mem::take(&mut doc.layers);
    let nodes = art_of(layer_art(&layers));
    let natural = shown_bounds(&doc.symbol_boxes(), &nodes)
        .filter(|r| r.width() > 0.0 || r.height() > 0.0)
        .ok_or_else(|| bad(cmd, format!("`{name}` has no art to place")))?;
    Ok(Loaded { name, format, art: Art::Vector { src: Box::new(doc), nodes, clip: None }, natural, warnings, link: None, board: None })
}

/// A PSD or PSB file's layers, read as `document.open` reads them into `doc`, as art to place: an
/// image for each layer and a group for each layer group, bottom first, with the layer's name,
/// visibility, opacity, blend mode and opacity mask. The art's natural box is the file's canvas,
/// where the merged image would lie, and the art is clipped to the canvas when layers reach past it.
/// None when no layer brought any art in.
fn psd_groups(mut doc: Document) -> Option<(Art, Rect)> {
    let canvas = doc.artboards.first()?.rect;
    let nodes = psd_objects(&std::mem::take(&mut doc.layers));
    // Hidden layers count too: showing one later must not draw past the canvas.
    let symbols = doc.symbol_boxes();
    let reached = nodes.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, reach(&symbols, n)))?;
    let past = reached.union(canvas) != canvas;
    Some((Art::Vector { src: Box::new(doc), nodes, clip: past.then_some(canvas) }, canvas))
}

/// The slices of the PSD or PSB file `bytes` (none without it) laid over `canvas`, the natural box
/// of its art; their warnings go to `warnings`.
fn psd_slices(bytes: Option<&[u8]>, canvas: Rect, warnings: &mut Vec<String>) -> Slices {
    let Some(bytes) = bytes else { return vec![] };
    let (slices, notes) = fileio::psd_slices(bytes, canvas);
    warnings.extend(notes);
    slices
}

/// The bounds of `n` and every object inside it, hidden ones included.
fn reach(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    match n.children() {
        Some(kids) => kids.iter().fold(None, |acc, k| vectorcraft_geom::union_opt(acc, reach(symbols, k))),
        None => n.geometric_bounds_with(symbols),
    }
}

/// The layers a PSD import made as objects: a layer of sublayers as a group of them, and a layer
/// holding one image as that image, each with the layer's name, visibility, opacity, blend mode,
/// isolation and opacity mask.
fn psd_objects(layers: &[Arc<Node>]) -> Vec<Node> {
    layers
        .iter()
        .map(|l| {
            let mut n = (**l).clone();
            let NodeKind::Layer { children, .. } = &l.kind else { return n };
            n.kind = match &children[..] {
                [c] if matches!(c.kind, NodeKind::Image(_)) => c.kind.clone(),
                _ => NodeKind::Group { children: psd_objects(children).into_iter().map(Arc::new).collect(), clip: false },
            };
            n
        })
        .collect()
}

/// Page `page` (1-based) of the PDF/.ai/.ait file `src` (`bytes`, read from `path` when there is
/// one) as art to place: clipped to the box `opts.crop` names, or (`bounding`) the bounds of its
/// art; read with `opts.password`. Only the page placed is read. A .ai saved without PDF
/// compatibility is read from its editing data, as it opens, and placed as an EPS is: the art on
/// artboard `page`, its linked images found from its folder. So is page 1 of a .ai or PDF whose
/// PDF part can't be read. `opened`, when given, is the PDF `bytes` already read
/// ([`vectorcraft_pdf::open_pdf`] with `opts`' password): its pages are read without reading the
/// file again.
pub(crate) fn pdf_art(
    src: &str,
    (bytes, opened): (&[u8], Option<&vectorcraft_pdf::Opened>),
    path: Option<&str>,
    page: usize,
    opts: fileio::LoadOptions,
    bounding: bool,
    cmd: &str,
) -> Result<document::DocumentArt> {
    let name = fileio::file_name(src);
    let index = page.checked_sub(1).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))?;
    let o = fileio::LoadOptions { crop: if bounding { CropTo::Crop } else { opts.crop }, ..opts };
    let page_document = |i: usize| match opened {
        Some(opened) => fileio::opened_page_document(bytes, opened, i, &o),
        None => fileio::page_document(bytes, i, &o),
    };
    let mut read = page_document(index);
    // Such a file can have more artboards than placeholder pages.
    if page > 1
        && read.is_err()
        && let Err(e) = page_document(0)
        && fileio::is_placeholder(&e)
    {
        read = Err(e);
    }
    let (mut doc, warnings, alone) = fileio::page_or_editing_data(src, bytes, &o, index, read).map_err(|e| bad(cmd, format!("{name}: {e}")))?;
    // Linked images show their files, found from its folder. Its placed documents can place this
    // file again: one already being read further up, or past the most nested reads, keeps its links
    // as stored, as a VectorCraft document's do.
    if alone && let Some(_reading) = document::Reading::enter(bytes) {
        super::links::resolve(&mut doc, super::links::LinkBase::file(path), false, false);
    }
    doc.drop_edit_modes();
    let layers = std::mem::take(&mut doc.layers);
    let symbols = doc.symbol_boxes();
    // The page placed is a PDF import's only one, with one layer; the editing data's art is
    // whatever lies on the artboard.
    let index = if alone { index } else { 0 };
    let n = doc.artboards.len();
    let pages = if n == 1 { "page" } else { "pages" };
    let board = doc.artboards.get(index).map(|a| a.rect).ok_or_else(|| bad(cmd, format!("page {page}: `{name}` has {n} {pages}")))?;
    let pages = if alone { &layers[..] } else { layers.get(index..=index).unwrap_or_default() };
    let nodes = on_artboard(&symbols, pages, board);
    let clip = (!bounding).then_some(board);
    let frame = clip.or_else(|| shown_bounds(&symbols, &nodes));
    let frame = frame.filter(|r| r.width() > 0.0 || r.height() > 0.0).ok_or_else(|| bad(cmd, format!("`{name}` has no art to place")))?;
    Ok(document::DocumentArt { doc, nodes, clip, frame, warnings, editing_data: alone })
}

/// The bounds of what `nodes` draw, as an export measures art
/// ([`vectorcraft_render::encode::drawn_bounds`]): hidden objects, guides and template layers
/// left out, symbol instances measured by their art in `symbols`.
pub(crate) fn shown_bounds(symbols: &SymbolBoxes, nodes: &[Node]) -> Option<Rect> {
    nodes.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, vectorcraft_render::encode::drawn_bounds(symbols, n)))
}

/// The placeable art of `layers` that lies on `board`: each object whose drawn bounds
/// ([`vectorcraft_render::encode::drawn_bounds`]) lie on it, or, for an object that draws nothing,
/// each object whose visual bounds lie on it.
fn on_artboard(symbols: &SymbolBoxes, layers: &[Arc<Node>], board: Rect) -> Vec<Node> {
    let on_board = |c: &&Arc<Node>| {
        let b = vectorcraft_render::encode::drawn_bounds(symbols, c).or_else(|| c.visual_bounds_with(symbols));
        b.is_some_and(|b| b.intersect(board).area() > 0.0 || board.contains(b.origin()))
    };
    art_of(layer_art(layers).filter(on_board))
}

/// The objects of the placeable `layers`, a clipping layer as itself: its objects alone would lose
/// their clip, and its clipping path would come in as a loose path.
fn layer_art(layers: &[Arc<Node>]) -> impl Iterator<Item = &Arc<Node>> {
    layers.iter().filter(|l| placeable(l)).flat_map(|l| match &l.kind {
        NodeKind::Layer { clip: true, .. } => std::slice::from_ref(l),
        _ => l.children().map_or(&[][..], Vec::as_slice),
    })
}

/// A clipping path of rectangle `r`, with id `id`.
fn clip_path(id: NodeId, r: Rect) -> Node {
    let mut c = Node::path(id, shapes::rectangle(r), Appearance::basic(Paint::None, Paint::None, 0.0));
    if let NodeKind::Path { clipping, .. } = &mut c.kind {
        *clipping = true;
    }
    c
}

/// A visible, non-template layer.
fn placeable(l: &Node) -> bool {
    l.visible && !matches!(l.kind, NodeKind::Layer { template: true, .. })
}

/// The placeable art among `nodes`: guides left out, visible sublayers as groups (clip groups for
/// clipping layers).
fn art_of<'a>(nodes: impl Iterator<Item = &'a Arc<Node>>) -> Vec<Node> {
    nodes
        .filter_map(|c| match &c.kind {
            _ if c.is_guide() => None,
            NodeKind::Layer { children, clip, .. } => placeable(c).then(|| {
                let mut g = (**c).clone();
                g.kind = NodeKind::Group { children: art_of(children.iter()).into_iter().map(Arc::new).collect(), clip: *clip };
                g
            }),
            _ => Some((**c).clone()),
        })
        .collect()
}

/// The art as one object of `d` (its resources added), at its natural size and position.
fn build(d: &mut Document, l: Loaded, link: Option<LinkInfo>) -> Node {
    let mut node = match l.art {
        Art::Image(img, clip) => {
            let (sx, sy) = pt_per_px(img.ppi);
            super::links::store_image(d, &img.key, img.blob, link.is_some());
            let im = ImageObject {
                key: img.key,
                width: img.width,
                height: img.height,
                xf: Affine::scale_non_uniform(sx, sy),
                link,
                placement: Default::default(),
                pixels: None,
            };
            let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
            fileio::file_clipped(d, image, clip)
        }
        Art::Dcs(a) => {
            let a = *a;
            super::links::store_image(d, &a.img.key, a.img.blob, link.is_some());
            dcs::add_spot_swatches(d, &a.spots);
            let im =
                ImageObject { key: a.img.key, width: a.img.width, height: a.img.height, xf: a.xf, link, placement: Default::default(), pixels: None };
            let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
            match a.clip {
                Some(c) => {
                    let children = vec![Arc::new(d.reid(&c)), Arc::new(image)];
                    Node::new(d.alloc_id(), NodeKind::Group { children, clip: true })
                }
                None => image,
            }
        }
        Art::Text(t) => Node::new(d.alloc_id(), NodeKind::Text(Box::new(text::area_text(t, l.natural, &d.setup)))),
        Art::Document(s, link, bounding) => document::node(d, &s, link, bounding),
        Art::Vector { src, mut nodes, clip } => {
            adopt::adopt(d, &src, &mut nodes);
            let mut children: Vec<Arc<Node>> = nodes.iter().map(|n| Arc::new(d.reid(n))).collect();
            if let Some(r) = clip {
                children.insert(0, Arc::new(clip_path(d.alloc_id(), r)));
            }
            Node::new(d.alloc_id(), NodeKind::Group { children, clip: clip.is_some() })
        }
    };
    node.name = Some(l.name);
    node
}

/// `node` moved and scaled by `m` as a whole: strokes, effects and pattern tiles go along.
pub(crate) fn transformed(mut node: Node, m: Affine) -> Node {
    node.transform(m, Scaling { strokes: true, effects: Some(vectorcraft_render::effects::scale_effect), ..Default::default() });
    adopt::each_mut(&mut node, &mut |n| vectorcraft_doc::pattern::transform_pattern_paints(n, m));
    node
}

// ---------- file.place ----------

/// `[x, y, width, height]` with a positive size.
fn rect_param(p: &Value) -> Result<Option<Rect>> {
    let Some(v) = p.get("rect") else { return Ok(None) };
    // All four or nothing: a malformed entry dropped would shift the others ([0, "bad", 10, 40, 50]).
    match finite_numbers(v) {
        Some([x, y, w, h]) if w > 0.0 && h > 0.0 => Ok(Some(Rect::new(x, y, x + w, y + h))),
        _ => Err(bad(PLACE, "rect must be [x, y, width, height] with a positive width and height")),
    }
}

/// `at`, the `[x, y]` the art is centred on (absent or null: none); anything else fails rather than
/// placing the art as if no point were given.
fn at_param(p: &Value) -> Result<Option<Point>> {
    p.get("at")
        .filter(|v| !v.is_null())
        .map(|v| finite_numbers(v).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(PLACE, "at must be [x, y]")))
        .transpose()
}

/// The transform that puts new art with the natural box `natural` where `old` is: an image's scale
/// and rotation relative to its 100% size (its resolution's) and its centre are kept (with
/// `image_xf`), also for an image alone in a clip group when the new art is an image, clipped by
/// its file's clipping path or not (`new_image`); other objects keep their centre.
fn replace_xf(d: &Document, old: &Node, natural: Rect, image_xf: bool, new_image: bool) -> Affine {
    let image = replace_measured(d, old, new_image);
    if image_xf && let NodeKind::Image(im) = &image.kind {
        let (sx, sy) = pt_per_px(d.images.get(&im.key).and_then(|b| fileio::ppi::resolution(&b.bytes)));
        let full = Affine::scale_non_uniform(sx, sy);
        let centre = full * Point::new(im.width as f64 / 2.0, im.height as f64 / 2.0);
        if im.xf.determinant().abs() > 1e-12 {
            return im.xf * full.inverse() * Affine::translate(centre - natural.center());
        }
    }
    let c = old.geometric_bounds_with(&d.symbol_boxes()).map_or(natural.center(), |b| b.center());
    Affine::translate(c - natural.center())
}

/// The object [`replace_xf`] measures when Replace swaps `old`: an image alone in a clip group (as
/// a PSD or PSB file's clipping path places it, [`fileio::file_clipped`]) when the new art is an
/// image (`new_image`), except a DCS file's plates; else `old` itself.
fn replace_measured<'a>(d: &Document, old: &'a Node, new_image: bool) -> &'a Node {
    match &old.kind {
        NodeKind::Group { children, clip: true } if new_image => match &children[..] {
            [path, image] if matches!(path.kind, NodeKind::Path { clipping: true, .. }) => match &image.kind {
                NodeKind::Image(im) if d.images.get(&im.key).is_none_or(|b| b.separated().is_none()) => image,
                _ => old,
            },
            _ => old,
        },
        _ => old,
    }
}

/// `rect` fitted with `natural`'s aspect, centred.
fn fit(natural: Rect, r: Rect) -> Affine {
    let ratio = |a: f64, b: f64| (b > 1e-9).then(|| a / b);
    let k = [ratio(r.width(), natural.width()), ratio(r.height(), natural.height())].into_iter().flatten().reduce(f64::min).unwrap_or(1.0);
    Affine::translate(r.center().to_vec2()) * Affine::scale(k) * Affine::translate(-natural.center().to_vec2())
}

/// What `file.place` takes from the session before it reads the file: the document it places into,
/// the params, the object Replace swaps and the artboard under `at`, which a DXF drawing is laid
/// out on ([`Session::prepare_place`]). [`PlaceJob::read`] reads and decodes the file without the
/// session, on any thread.
pub struct PlaceJob {
    /// The document it was prepared in (`DocState::uid`).
    doc: u64,
    params: Value,
    old: Option<NodeId>,
    /// The object Replace swaps as it was when Place started.
    replaced: Option<Node>,
    target: Option<Rect>,
}

/// A file [`PlaceJob::read`] read, which [`Session::place_read`] places.
pub struct PlaceRead {
    job: PlaceJob,
    loaded: Loaded,
    slices: Slices,
}

impl PlaceJob {
    /// The name of the file it places ("photo.tif").
    pub fn name(&self) -> String {
        str_param(&self.params, "path").or_else(|| str_param(&self.params, "name")).map(fileio::file_name).unwrap_or_else(|| "Untitled".into())
    }

    /// Read and decode the file, as `file.place` does, without the session. The pixels of a PNG,
    /// JPEG, GIF or WebP file are decoded here to check that they can be read (an error when they
    /// can't), a linked image's preview is made from that decode, and the decoded pixels are kept in
    /// the renderers' image store for the first frame that draws them
    /// ([`super::links::readable_and_kept`]).
    pub fn read(self) -> Result<PlaceRead> {
        crate::cancel::check()?;
        let (mut loaded, slices) = load_sliced(&self.params, PLACE, self.target, true)?;
        let linked = loaded.link.is_some() && bool_or(&self.params, "link", true);
        match &mut loaded.art {
            Art::Image(img, _) => {
                let unread = super::links::kept_unread(&img.blob.bytes);
                img.blob = super::links::readable_and_kept(img.blob.clone(), unread, linked)?;
            }
            Art::Dcs(a) => a.img.blob = super::links::readable(a.img.blob.clone(), false, true)?,
            _ => {}
        }
        Ok(PlaceRead { job: self, loaded, slices })
    }
}

impl Session {
    /// `file.place`'s checks (with no document open, the error `file.place` gives) of `params`
    /// against the active document as it is now → what reads the file ([`PlaceJob::read`]; on the
    /// desktop File › Place does this off the UI thread). Replace swaps the object selected now;
    /// the art is placed against the artboard under `at` as it is when [`Session::place_read`]
    /// places it.
    pub fn prepare_place(&self, params: &Value) -> Result<PlaceJob> {
        has_doc(self).map_err(|why| EngineError::Disabled(PLACE.into(), why))?;
        prepare(self, params)
    }

    /// Run `file.place` with the params of `read` as a command (checked and journaled as any
    /// other) that places the file `read` holds instead of reading it again → `file.place`'s
    /// reply. The document Place started in must be the active one, and the object Replace swaps
    /// must still be in it as it was when Place started; when the selection changed meanwhile,
    /// `select.set` selects that object again first, so a replay of the journal replaces the same
    /// object.
    pub fn place_read(&mut self, read: PlaceRead) -> Result<Value> {
        if self.doc()?.uid != read.job.doc {
            return Err(bad(PLACE, NOT_ACTIVE));
        }
        let params = read.job.params.clone();
        if let Some(old) = read.job.old {
            let st = self.doc()?;
            // Deleted, changed, or another object with its id: an undo that removes the object frees
            // its id for the next object made.
            if st.doc.node(old) != read.job.replaced.as_ref() {
                return Err(bad(PLACE, "replace: the object to replace was changed or deleted"));
            }
            if st.selection.objects != [old] || st.selection.symbol_part.is_some() {
                self.execute("select.set", &json!({ "ids": [old.0] }))?;
            }
        }
        self.place_hand_off = Some(read);
        let r = self.execute(PLACE, &params);
        self.place_hand_off = None;
        r
    }
}

fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let read = match s.place_hand_off.take() {
        Some(read) => read,
        None => prepare(s, p)?.read()?,
    };
    apply(s, read)
}

fn prepare(s: &Session, p: &Value) -> Result<PlaceJob> {
    let replace = switch_or(p, "replace", false, PLACE)?;
    let template = switch_or(p, "template", false, PLACE)?;
    check_switches(p, &["link"], PLACE)?;
    let at = at_param(p)?;
    let rect = rect_param(p)?;
    if replace && (template || at.is_some() || rect.is_some()) {
        return Err(bad(PLACE, "replace keeps the replaced object's place and transform: drop template, at and rect"));
    }
    let corners = [at, rect.map(|r| r.origin()), rect.map(|r| Point::new(r.x1, r.y1))];
    if corners.iter().flatten().any(|c| !(c.x.abs() <= MAX_COORD && c.y.abs() <= MAX_COORD)) {
        return Err(bad(PLACE, "at/rect lie outside the canvas"));
    }
    // An object selected in a symbol instance's art can't be swapped, and its instance stays.
    if replace {
        super::edit::refuse_symbol_part(s)?;
    }
    let st = s.doc()?;
    let old = match (replace, &st.selection.objects[..]) {
        (false, _) => None,
        (true, [id]) => Some(*id),
        (true, _) => return Err(bad(PLACE, "replace: select the one object to replace")),
    };
    let replaced = old.and_then(|id| st.doc.node(id)).cloned();
    Ok(PlaceJob { doc: st.uid, params: s.prefs.with_pdf_font_choice(p), old, replaced, target: board_at(&st.doc, at) })
}

/// Place the file `read` holds in the active document.
fn apply(s: &mut Session, read: PlaceRead) -> Result<Value> {
    let PlaceRead { job: PlaceJob { params, old, target: read_for, .. }, mut loaded, slices: read_slices } = read;
    let p = &params;
    let template = bool_or(p, "template", false);
    let at = at_param(p)?;
    let rect = rect_param(p)?;
    let st = s.doc()?;
    let old_node = match old {
        Some(id) => Some(st.doc.node(id).ok_or_else(|| bad(PLACE, "replace: the object to replace was changed or deleted"))?),
        None => None,
    };
    // The artboard under `at` as it is now, as a replay of the journal finds it: a DXF drawing laid
    // out on the artboard it was read for is read again when that artboard changed meanwhile.
    let target = board_at(&st.doc, at);
    if loaded.board.is_some() && target != read_for {
        loaded = load(p, PLACE, target, true)?;
    }
    // Type reflows rather than scales: its frame takes the size it is placed at.
    let is_text = matches!(loaded.art, Art::Text(_));
    if is_text {
        let size = match (old_node.and_then(|o| o.geometric_bounds_with(&st.doc.symbol_boxes())), rect) {
            (Some(b), _) => b.size(),
            (None, Some(r)) => r.size(),
            (None, None) => text_frame(target).size(),
        };
        loaded.natural = Rect::from_origin_size(Point::ORIGIN, size);
    }
    let natural = loaded.natural;
    // Replace keeps the transform of the image it measures (`replace_measured`), which a
    // distortion's corners do not follow. Type, and any other object it measures, is placed by the
    // bounds of what it replaces, so a distorted image there is not refused.
    if let Some(o) = old_node
        && !is_text
    {
        let image = replace_measured(&st.doc, o, matches!(loaded.art, Art::Image(..)));
        if image.is_distorted_image() {
            return Err(super::xform::distorted_image_error(PLACE, image.id));
        }
    }
    let m = match (old_node, rect) {
        (Some(o), _) => replace_xf(&st.doc, o, natural, !is_text, matches!(loaded.art, Art::Image(..))),
        (None, Some(r)) => fit(natural, r),
        // Its artboard's bottom-left corner on the target's.
        (None, None) if let (Some(src), Some(to)) = (loaded.board, target) => Affine::translate((to.x0 - src.x0, to.y1 - src.y1)),
        (None, None) => {
            let c = at.or_else(|| st.doc.artboards.first().map(|a| a.rect.center())).unwrap_or_default();
            Affine::translate(c - natural.center())
        }
    };
    if !crate::sanity::rect_in_range(m.transform_rect_bbox(natural)) {
        return Err(bad(PLACE, "placed art lies outside the canvas"));
    }
    let parent = st.insertion_parent();
    let below = st.current_layer().or_else(|| parent.and_then(|p| st.doc.layer_of(p)));
    let link = loaded.link.take().filter(|_| bool_or(p, "link", true));
    let linked = link.is_some();
    let (name, format, mut warnings) = (loaded.name.clone(), loaded.format.id, std::mem::take(&mut loaded.warnings));
    let file_clipped = matches!(&loaded.art, Art::Dcs(a) if a.clip.is_some()) || matches!(&loaded.art, Art::Image(_, Some(_)));
    if let Art::Dcs(a) = &loaded.art {
        warnings.extend(dcs::swatch_notes(&st.doc, &a.spots));
    }
    // A PSD file's slices move and scale with its art; one that would leave the canvas is left out.
    let read_count = read_slices.len();
    let slices: Slices = read_slices
        .into_iter()
        .filter_map(|(r, o)| {
            let corners = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)].map(|c| m * c);
            let placed = corners.iter().skip(1).fold(Rect::from_points(corners[0], corners[0]), |acc, c| acc.union_pt(*c));
            super::slices::slice_rect_ok(placed).then_some((placed, o))
        })
        .collect();
    match read_count - slices.len() {
        0 => {}
        1 => warnings.push("1 slice would lie outside the canvas and was left out".into()),
        n => warnings.push(format!("{n} slices would lie outside the canvas and were left out")),
    }
    let (id, size, slice_ids) = s.edit("Place", |d, sel| {
        let node = transformed(build(d, loaded, link), m);
        let size = node.geometric_bounds_with(&d.symbol_boxes()).unwrap_or_default();
        let (into, index) = match old {
            Some(old) => {
                let (par, idx, _) = d.position(old).ok_or(EngineError::NoNode(old))?;
                d.remove(old)?;
                (par, idx)
            }
            None if template => (Some(template_layer(d, below, &name)?), usize::MAX),
            None => (parent, usize::MAX),
        };
        let id = d.insert(into, index, node)?;
        let slice_ids: Vec<u64> = slices.into_iter().map(|(r, o)| super::slices::add_slice(d, r, o).0).collect();
        // Template art is locked: nothing to select.
        if template {
            sel.clear()
        } else {
            sel.set([id])
        }
        Ok((id, size, slice_ids))
    })?;
    let mut out = json!({ "ids": [id.0], "name": name, "format": format, "linked": linked, "width": size.width(), "height": size.height(), "warnings": warnings });
    if !slice_ids.is_empty() {
        out["slices"] = json!(slice_ids);
    }
    // A DCS, PSD or PSB file clipped by its clipping path: its image, which the Links commands
    // take.
    if file_clipped {
        let image = s.doc()?.doc.node(id).and_then(|n| match &n.kind {
            NodeKind::Group { children, clip: true } => children.get(1).filter(|c| matches!(c.kind, NodeKind::Image(_))).map(|c| c.id),
            _ => None,
        });
        if let Some(image) = image {
            out["image"] = json!(image.0);
        }
    }
    Ok(out)
}

/// The artboard under `at` (its edges included, [`Document::artboard_holding`]), else the first.
fn board_at(d: &Document, at: Option<Point>) -> Option<Rect> {
    at.and_then(|c| d.artboard_holding(c, false)).and_then(|i| d.artboards.get(i)).or(d.artboards.first()).map(|a| a.rect)
}

/// Placed type's frame without `rect` or Replace: the artboard `board` (the one under `at`, else the
/// first, [`board_at`]) less a 36 pt margin.
fn text_frame(board: Option<Rect>) -> Rect {
    let board = board.unwrap_or(TEXT_FRAME);
    Rect::from_center_size(board.center(), ((board.width() - 72.0).max(72.0), (board.height() - 72.0).max(72.0)))
}

/// A new locked template layer named after the file, right below the layer `below` (else at the
/// bottom).
fn template_layer(d: &mut Document, below: Option<NodeId>, name: &str) -> Result<NodeId> {
    let id = d.add_layer(Some(&format!("Template {name}")));
    let at = below.and_then(|b| d.layers.iter().position(|l| l.id == b)).unwrap_or(0);
    d.move_node(id, None, at)?;
    if let Some(n) = d.node_mut(id) {
        n.locked = true;
        if let NodeKind::Layer { template, .. } = &mut n.kind {
            *template = true;
        }
    }
    Ok(id)
}

// ---------- file.place.queue ----------

/// The files `file.place.queue` names, as `file.place` params.
fn queued_files(p: &Value) -> Result<Vec<Value>> {
    let list = |k: &str| p.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut out = vec![];
    for v in list("paths") {
        let path = v.as_str().ok_or_else(|| bad(QUEUE, "paths must be strings"))?;
        out.push(json!({ "path": path }));
    }
    for f in list("files") {
        let (Some(name), Some(data)) = (str_param(f, "name"), str_param(f, "dataBase64")) else {
            return Err(bad(QUEUE, "files must be {name, dataBase64}"));
        };
        out.push(json!({ "name": name, "dataBase64": data }));
    }
    match out.len() {
        0 => Err(bad(QUEUE, "give paths or files")),
        n if n > MAX_QUEUE => Err(bad(QUEUE, format!("at most {MAX_QUEUE} files"))),
        _ => Ok(out),
    }
}

/// The files `file.place.queue` loads the place cursor with, before they are read
/// ([`Session::prepare_place_queue`]). [`QueueJob::read`] reads them without the session, on any
/// thread.
pub struct QueueJob {
    /// The document it was prepared in (`DocState::uid`).
    doc: u64,
    params: Value,
    files: Vec<Value>,
}

/// The files [`QueueJob::read`] read, which [`Session::place_queue_read`] loads the place cursor
/// with.
pub struct QueueRead {
    /// The document the files were read for (`DocState::uid`).
    doc: u64,
    params: Value,
    entries: Vec<Value>,
    files: Vec<Value>,
    skipped: Vec<Value>,
}

impl QueueJob {
    /// The names of the files it loads ("a.tif", "b.psd").
    pub fn names(&self) -> Vec<String> {
        self.files.iter().filter_map(|q| str_param(q, "path").or(str_param(q, "name"))).map(fileio::file_name).collect()
    }

    /// Read every file for its size and thumbnail, as `file.place.queue` does, without the session.
    pub fn read(self) -> Result<QueueRead> {
        read_queue(self.doc, self.params, self.files)
    }
}

/// Read each of `queued`, the files of `file.place.queue` with params `p` for the document `doc`,
/// for its size and thumbnail.
fn read_queue(doc: u64, p: Value, queued: Vec<Value>) -> Result<QueueRead> {
    let thumbnail = p.get("thumbnail").and_then(Value::as_f64);
    let (mut entries, mut files, mut skipped) = (vec![], vec![], vec![]);
    for mut q in queued {
        for k in ["link", "template", "page", "crop", "text", "dxf", "layers", "hiddenLayers", "importSlices", "importer", "params"] {
            if let Some(v) = p.get(k) {
                q[k] = v.clone();
            }
        }
        // Read for its size and thumbnail: each click places it with file.place, so a file whose
        // pixels file.place can't read is skipped. With a thumbnail, the decoded pixels are kept
        // so that drawing it does not decode the file again.
        let checked = load(&q, QUEUE, None, false).and_then(|mut l| {
            let linked = l.link.is_some() && bool_or(&q, "link", true);
            match &mut l.art {
                Art::Image(img, _) => {
                    let unread = super::links::kept_unread(&img.blob.bytes);
                    let check = if thumbnail.is_some() { super::links::readable_and_kept } else { super::links::readable };
                    img.blob = check(img.blob.clone(), unread, linked)?;
                }
                Art::Dcs(a) => a.img.blob = super::links::readable(a.img.blob.clone(), false, true)?,
                _ => {}
            }
            Ok(l)
        });
        match checked {
            Ok(l) => {
                let (w, h) = (l.natural.width(), l.natural.height());
                let mut f = json!({ "name": l.name, "format": l.format.id, "width": w, "height": h });
                entries.push(json!({ "params": q, "name": l.name, "width": w, "height": h }));
                if let Some(px) = thumbnail {
                    f["thumbnailBase64"] = json!(thumbnail_png(l, px)?);
                }
                files.push(f);
            }
            Err(e) => {
                let name = str_param(&q, "path").or(str_param(&q, "name")).map(fileio::file_name).unwrap_or_default();
                // An encrypted PDF stays, at a size of its own, so that a click in the app can ask
                // for its password.
                if let Some(format) = locked_pdf(&q, &e) {
                    let (w, h) = (LOCKED_PAGE.width(), LOCKED_PAGE.height());
                    files.push(json!({ "name": name, "format": format, "width": w, "height": h, "needsPassword": true }));
                    entries.push(json!({ "params": q, "name": name, "width": w, "height": h }));
                    continue;
                }
                skipped.push(json!({ "name": name, "error": e.to_string() }));
            }
        }
    }
    Ok(QueueRead { doc, params: p, entries, files, skipped })
}

/// The format id of the file `q` names when it is a PDF, `.ai` or `.ait` file (a path with one of
/// those extensions, as the app's password dialog reads it) that opens only with a password, and
/// reading it failed with `e` for want of that password (not for a bad option).
fn locked_pdf(q: &Value, e: &EngineError) -> Option<&'static str> {
    if !e.to_string().contains(&vectorcraft_pdf::PdfError::NeedsPassword.to_string()) {
        return None;
    }
    let src = fileio::source(q, QUEUE).ok()?;
    if src.path.is_some() && !matches!(fileio::extension(src.name).as_str(), "pdf" | "ai" | "ait") {
        return None;
    }
    let format = fileio::detect(src.name, &src.bytes)?;
    let locked = matches!(vectorcraft_pdf::open_pdf(&src.bytes, None).map(|_| ()), Err(vectorcraft_pdf::PdfError::NeedsPassword));
    (matches!(format.id, "pdf" | "ai" | "ait") && locked).then_some(format.id)
}

impl Session {
    /// `file.place.queue`'s checks (with no document open, the error `file.place.queue` gives) of
    /// `params` → what reads the files ([`QueueJob::read`]; on the
    /// desktop the Place dialog's OK with several files does this off the UI thread).
    pub fn prepare_place_queue(&self, params: &Value) -> Result<QueueJob> {
        has_doc(self).map_err(|why| EngineError::Disabled(QUEUE.into(), why))?;
        check_switches(params, &["link", "template", "layers", "hiddenLayers", "importSlices"], QUEUE)?;
        check_numbers(params, &["thumbnail"], QUEUE)?;
        let files = queued_files(params)?;
        Ok(QueueJob { doc: self.doc()?.uid, params: params.clone(), files })
    }

    /// Run `file.place.queue` with the params of `read` as a command that loads the place cursor
    /// with the files `read` holds instead of reading them again → `file.place.queue`'s reply. The
    /// document the files were read for must be the active one.
    pub fn place_queue_read(&mut self, read: QueueRead) -> Result<Value> {
        if self.doc()?.uid != read.doc {
            return Err(bad(QUEUE, NOT_ACTIVE));
        }
        let params = read.params.clone();
        self.queue_hand_off = Some(read);
        let r = self.execute(QUEUE, &params);
        self.queue_hand_off = None;
        r
    }
}

fn queue(s: &mut Session, p: &Value) -> Result<Value> {
    let read = match s.queue_hand_off.take() {
        Some(read) => read,
        None => s.prepare_place_queue(p)?.read()?,
    };
    let QueueRead { entries, files, skipped, .. } = read;
    if entries.is_empty() {
        let why = skipped.first().and_then(|e| e["error"].as_str()).unwrap_or("nothing to place");
        return Err(bad(QUEUE, why));
    }
    // Reloading the cursor keeps the tool to return to.
    let prev = Some(s.tool_id()).filter(|t| *t != "place");
    let view = s.last_view;
    s.select_tool("place", view)?;
    s.set_tool_option("queue", &json!({ "entries": entries, "prev": prev }));
    Ok(json!({ "count": files.len(), "files": files, "skipped": skipped }))
}

// ---------- queries ----------

/// The colour mode an image file stores its pixels in.
fn color_mode(bytes: &[u8]) -> &'static str {
    match craft_image_codecs::decode(bytes).ok().map(|img| img.layout()) {
        Some(craft_image_codecs::ChannelLayout::Gray | craft_image_codecs::ChannelLayout::GrayA) => "Grayscale",
        Some(craft_image_codecs::ChannelLayout::Cmyk | craft_image_codecs::ChannelLayout::CmykA) => "CMYK",
        _ => "RGB",
    }
}

/// `l` rendered as a PNG at most `px` (≤ [`MAX_THUMBNAIL`]) pixels on its longer side, base64.
fn thumbnail_png(l: Loaded, px: f64) -> Result<String> {
    let natural = l.natural;
    let mut d = Document::new(natural.width().max(1.0), natural.height().max(1.0));
    let layer = d.layers.first().map(|l| l.id);
    let node = build(&mut d, l, None);
    d.insert(layer, 0, node)?;
    let k = px.clamp(1.0, MAX_THUMBNAIL) / natural.width().max(natural.height()).max(1e-9);
    let png = vectorcraft_render::Renderer::new().render_region(&d, natural, k, false).to_png().map_err(EngineError::Other)?;
    Ok(vectorcraft_format::base64_encode(&png))
}

fn info(_: &mut Session, p: &Value) -> Result<Value> {
    read_info(p)
}

/// `file.place.info` with `p`, without the session (on the desktop the Place dialog reads its
/// files' lines this way off the UI thread).
pub fn read_info(p: &Value) -> Result<Value> {
    check_switches(p, &["link"], "file.place.info")?;
    check_numbers(p, &["thumbnail"], "file.place.info")?;
    // A PDF/.ai page linked or not has the same size and art.
    let (mut l, slices) = load_sliced(p, "file.place.info", None, false)?;
    let natural = l.natural;
    let mut out = json!({
        "name": l.name,
        "format": l.format.id,
        "width": natural.width(),
        "height": natural.height(),
        "warnings": std::mem::take(&mut l.warnings),
    });
    if matches!(l.format.id, "psd" | "psb") {
        out["slices"] = json!(slices.len());
    }
    if let Art::Image(img, _) = &l.art {
        let (x, y) = img.ppi.unwrap_or((72.0, 72.0));
        out["pixelWidth"] = json!(img.width);
        out["pixelHeight"] = json!(img.height);
        out["ppi"] = json!([x, y]);
        out["colorMode"] = json!(color_mode(&img.blob.bytes));
    }
    if let Art::Dcs(a) = &l.art {
        let (x, y) = a.img.ppi.unwrap_or((72.0, 72.0));
        out["pixelWidth"] = json!(a.img.width);
        out["pixelHeight"] = json!(a.img.height);
        out["ppi"] = json!([x, y]);
        out["colorMode"] = json!("CMYK");
        let plates: Vec<String> = a.img.blob.separated().map(|s| s.plates.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
        out["plates"] = json!(plates);
    }
    if let Some(px) = p.get("thumbnail").and_then(Value::as_f64) {
        out["thumbnailBase64"] = json!(thumbnail_png(l, px)?);
    }
    Ok(out)
}

/// Resolution at a placed image's transform: 72 points per inch over each pixel axis.
pub(super) fn effective_ppi(xf: Affine) -> [f64; 2] {
    let [a, b, c, d, ..] = xf.as_coeffs();
    let ppi = |x: f64, y: f64| Some(x.hypot(y)).filter(|l| *l > 1e-12).map_or(0.0, |l| 72.0 / l);
    [ppi(a, b), ppi(c, d)]
}

pub(crate) fn image_info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "image.info";
    let st = s.doc()?;
    let id = match (object_param(p, "id", C)?, &st.selection.objects[..]) {
        (Some(id), _) => id,
        (None, [id]) => *id,
        _ => return Err(bad(C, "select one image or pass id")),
    };
    // An image in opacity-mask art, a definition's art or a stored brush's art is found by its id
    // too (`links.list`).
    let doc = super::links::with_brush_art(&st.doc);
    let n = doc.stored_node(id).map(|(n, _)| &**n).ok_or(EngineError::NoNode(id))?;
    let NodeKind::Image(im) = &n.kind else { return Err(bad(C, format!("object {} is not an image", id.0))) };
    // Effective resolution at the middle of a distorted image.
    let xf = vectorcraft_doc::image_quad::nearest_affine(n).unwrap_or(im.xf);
    let size = n.geometric_bounds().unwrap_or_default();
    // A linked file outside the read roots of the caller asking now is left unnamed, as in Link Info.
    let doc_path = super::links::LinkBase::of(st).doc;
    Ok(json!({
        "id": id.0,
        "name": n.display_name(),
        "linked": im.link.is_some(),
        "link": im.link.as_ref().map(|l| &l.path).filter(|path| super::links::path_shown(path, doc_path)),
        "colorMode": st.doc.images.get(&im.key).map_or("RGB", |b| color_mode(&b.bytes)),
        "pixelWidth": im.width,
        "pixelHeight": im.height,
        "ppi": effective_ppi(xf),
        "width": size.width(),
        "height": size.height(),
    }))
}
