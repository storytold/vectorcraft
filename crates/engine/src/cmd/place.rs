//! File → Place: another file's art into the active document as one undo step that never touches
//! the clipboard.
//!
//! - A raster image becomes one image at 100% of its physical size: its pixels at the resolution
//!   the file declares ([`fileio::ppi`]; 72 ppi when it declares none), linked to its file with
//!   `link` ([`super::links`]).
//! - An SVG becomes one group of its art; so does a DXF drawing, read with the DXF options
//!   (`dxf`, [`fileio::dxfimport`]), and an HP-GL plot file (`vectorcraft_cad::hpgl`).
//! - An EMF, WMF or PICT picture becomes one group, clipped to its frame (`crop: "crop"`) or
//!   bounded by its art (`crop: "bounding"`).
//! - An EPS file (or a PostScript .ai) becomes one group, clipped to its bounding box or bounded by
//!   its art: the document an EPS file of ours carries, else what its PostScript draws, else its
//!   preview ([`fileio::load`]).
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
use vectorcraft_doc::{Appearance, Document, ImageObject, LinkInfo, Node, NodeId, NodeKind, PageBox, Scaling, SymbolBoxes};
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
/// The most files one `file.place.queue` loads.
const MAX_QUEUE: usize = 100;
/// The longest thumbnail side `thumbnail` may ask for (px).
const MAX_THUMBNAIL: f64 = 512.0;
/// A text file's natural box (`file.place.info`, the place cursor): a letter page less 1 in margins.
const TEXT_FRAME: Rect = Rect::new(0.0, 0.0, 468.0, 648.0);

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.place",
            "Place…",
            ["File"],
            Some("Cmd+Shift+P"),
            "{path | name | dataBase64 (a file's path, or its name with its bytes), link?: true (a raster image keeps its file's path; a VectorCraft document at path becomes one locked placed document showing artboard `page`, linked to the file and read again when it changes (links.update; saving it in this app updates it at once), drawn as vectors in every output; so does a PDF/.ai page at path, showing its `crop` box, unless password or textAs: \"outlines\" is given; false: an editable copy of its art, as from dataBase64; other files are embedded), text?: {characterSet?: \"unicode\" (UTF-8, or UTF-16 with a byte-order mark; other bytes as the platform's 8-bit set) | \"ansi\" (the platform's 8-bit set), platform?: \"windows\" (Windows-1252) | \"mac\" (Mac Roman), removeLineReturns?: false (each block of lines becomes one paragraph; blank lines end paragraphs), removeParagraphReturns?: false (drop blank lines), replaceSpaces?: n (runs of n ≥ 2 spaces become a tab)} (a .txt file, placed as area type filling rect, the replaced object's bounds, or else the artboard less a 36 pt margin), template?: false (onto a new locked template layer below the current layer), replace?: false (swap the one selected object, keeping its stacking place and transform; no at/rect; refused while an object of a symbol instance's art is selected), at?: [x, y] centre (default: the first artboard's centre), rect?: [x, y, width, height] fit inside; aspect kept (wins over at), pages?: \"all\" (PDF: place every page side-by-side, 20 pt apart) | page?: 1 (PDF/.ai page, or a native document's artboard), crop?: \"crop\" (clipped to that page or artboard, default; an SVG, only when given: framed and clipped by its viewBox, else by its art's bounds) | \"bounding\" (the bounds of the art that shows, with its strokes and effects, as an export measures them: hidden objects and guides left out) | \"art\" | \"trim\" | \"bleed\" | \"media\" (a PDF page's boxes), password? (an encrypted PDF), dxf?: {…the DXF options of document.open} (a .dxf drawing: fit fills the artboard under at, else the first; fitted or uncentred art lands with its drawing's artboard on that artboard, else it is centred on at)} → {ids, name, format, linked, width, height, warnings, image?: the id of the image of a DCS file placed in a clip group, which the Links commands take}. Raster images come in at 100% of their physical size (the file's ppi, else 72); SVG, DXF, HP-GL (.plt, .hpgl, .hgl, .hpg), PDF/.ai, EMF/WMF and PICT (.pct, .pic, .pict; clipped to the picture's frame), EPS (clipped to its bounding box; as document.open reads it), a .ai saved without PDF compatibility (from its editing data, as document.open reads it: the art of the layers that show on artboard `page`, clipped to that artboard unless crop is \"bounding\") and native documents as one group, with the images, symbols, patterns, swatches and brushes they use. A DCS file (an EPS file with one program per printing plate, DCS 1.0 or 2.0) is placed only from its path with link (else an error, as are document.open and links.embed): one linked image of its plates mixed into CMYK, clipped by the clipping path its composite is drawn through, its spot plates added as spot swatches in the process equivalents the file gives (a swatch of the same name is kept, with a warning); print separations and Separations Preview show each plate on its own ink, and plate files of their own are read from its folder. One undo step; selects what it placed (unless on a template layer); never touches the clipboard. A PNG, JPEG, GIF or WebP image whose pixels can't be read (a file cut short) is an error, and nothing is placed",
            has_doc,
            place
        ),
        cmd!(
            query "file.place.info",
            "Placed File Info",
            [],
            None,
            "{path | name | dataBase64 (a file's path, or its name with its bytes), page?, crop?, password?, thumbnail?: px, link?: bool, dxf?: {…} (both as file.place)} what file.place would place, without placing it → {name, format, width, height (pt at 100%), pixelWidth?, pixelHeight?, ppi?: [x, y], colorMode?: RGB|Grayscale|CMYK (raster images), plates?: [ink names] (a DCS file), warnings, thumbnailBase64?: PNG of at most `thumbnail` (≤ 512) px on its longer side}",
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
            "{paths?: [path…], files?: [{name, dataBase64}…], link?, template?, page?, crop?, text?, dxf?, thumbnail?: px} load the place cursor (the `place` tool) with up to 100 files (as file.place reads them): a click places the current file at 100% with its top-left corner there, a drag places it at the dragged size (aspect kept), ←/→ and ↑/↓ cycle the files, Esc discards the current one; each placement is a file.place, and the previous tool returns after the last → {count, files: [{name, format, width, height, thumbnailBase64?}], skipped: [{name, error}]}",
            has_doc,
            queue
        ),
    ]
}

// ---------- reading the file ----------

/// A file's art at its natural size, before it joins a document.
enum Art {
    Image(RasterImage),
    /// A text file's, an RTF file's or a Word document's text, placed as area type filling the
    /// natural box.
    Text(text::Story),
    /// A VectorCraft document or a PDF/.ai page placed linked: the file read, the link, and
    /// whether the art's bounds (not the artboard or page) are shown.
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

/// Points per pixel of a raster image at the resolution it declares (72 ppi when none).
pub(crate) fn pt_per_px(ppi: Option<(f64, f64)>) -> (f64, f64) {
    let (x, y) = ppi.unwrap_or((72.0, 72.0));
    (72.0 / x, 72.0 / y)
}

/// Maximum number of PDF pages placed in a single `file.place` operation.
pub(crate) const MAX_PLACE_PAGES: usize = 64;
/// Maximum cumulative vector nodes imported across a single multipage `file.place` operation.
pub(crate) const MAX_PLACE_TOTAL_NODES: usize = 100_000;

/// Parses and validates `"page"` and `"pages"` parameters.
/// When `single_only` is true (`file.place.info` / `read_queue`), returns at most 1 page
/// without requiring later pages to be decoded.
fn resolve_pdf_pages(p: &Value, cmd: &str, single_only: bool, total_pages: Option<usize>) -> Result<Vec<usize>> {
    let has_page = p.get("page").is_some_and(|v| !v.is_null());
    let has_pages = p.get("pages").is_some_and(|v| !v.is_null());

    if has_page && has_pages {
        return Err(bad(cmd, "specify either page or pages, not both"));
    }

    if let Some(v) = p.get("page").filter(|v| !v.is_null()) {
        let n = v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))? as usize;
        if let Some(total) = total_pages.filter(|&total| n > total) {
            let pages = if total == 1 { "page" } else { "pages" };
            return Err(bad(cmd, format!("page {n}: has {total} {pages}")));
        }
        return Ok(vec![n]);
    }

    if let Some(v) = p.get("pages").filter(|v| !v.is_null()) {
        let raw =
            v.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "pages must be \"all\" or a page range such as \"1-3,5\""))?;

        if raw.eq_ignore_ascii_case("all") {
            if single_only {
                return Ok(vec![1]);
            }
            let total = total_pages.unwrap_or(1).max(1);
            if total > MAX_PLACE_PAGES {
                return Err(bad(
                    cmd,
                    format!(
                        "PDF has {total} pages, exceeding the {MAX_PLACE_PAGES}-page placement limit; pass a pages range such as \"1-{MAX_PLACE_PAGES}\""
                    ),
                ));
            }
            return Ok((1..=total).collect());
        }

        let parsed = parse_page_range_strict(raw, total_pages, cmd)?;
        if single_only {
            return parsed.into_iter().next().map(|first| vec![first]).ok_or_else(|| bad(cmd, "pages range resolved to no pages"));
        }
        if parsed.len() > MAX_PLACE_PAGES {
            return Err(bad(cmd, format!("requested {} pages, exceeding the {MAX_PLACE_PAGES}-page placement limit", parsed.len())));
        }
        return Ok(parsed);
    }

    // Neither "page" nor "pages" was provided:
    if single_only {
        return Ok(vec![1]);
    }
    let total = total_pages.unwrap_or(1).max(1);
    if total > MAX_PLACE_PAGES {
        return Err(bad(
            cmd,
            format!(
                "PDF has {total} pages, exceeding the {MAX_PLACE_PAGES}-page placement limit; pass a pages range such as \"1-{MAX_PLACE_PAGES}\""
            ),
        ));
    }
    Ok((1..=total).collect())
}

fn parse_page_range_strict(raw: &str, total_pages: Option<usize>, cmd: &str) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for part in raw.split(',') {
        let token = part.trim();
        if token.is_empty() {
            return Err(bad(cmd, format!("invalid pages range: \"{raw}\"")));
        }
        if let Some((a, b)) = token.split_once('-') {
            let start: usize = a.trim().parse().map_err(|_| bad(cmd, format!("invalid page number in pages: \"{token}\"")))?;
            let end: usize = b.trim().parse().map_err(|_| bad(cmd, format!("invalid page number in pages: \"{token}\"")))?;
            if start == 0 || end < start || end > 100_000 {
                return Err(bad(cmd, format!("invalid page range \"{token}\"")));
            }
            if let Some(total) = total_pages.filter(|&total| end > total) {
                return Err(bad(cmd, format!("page range \"{token}\" exceeds PDF page count ({total})")));
            }
            for page in start..=end {
                if seen.insert(page) {
                    out.push(page);
                }
            }
        } else {
            let page: usize = token.parse().map_err(|_| bad(cmd, format!("invalid page number in pages: \"{token}\"")))?;
            if page == 0 || page > 100_000 {
                return Err(bad(cmd, format!("invalid page number \"{token}\"")));
            }
            if let Some(total) = total_pages.filter(|&total| page > total) {
                return Err(bad(cmd, format!("page {page} exceeds PDF page count ({total})")));
            }
            if seen.insert(page) {
                out.push(page);
            }
        }
    }
    if out.is_empty() {
        return Err(bad(cmd, "pages range cannot be empty"));
    }
    Ok(out)
}

/// Direct single-page loader used by `file.place.info` (`read_info`) and `read_queue`.
/// Never decodes pages 2..=N when querying or previewing a file.
fn load(p: &Value, cmd: &str, board: Option<Rect>, link_pdf: bool) -> Result<Loaded> {
    let mut pages = load_pages_mode(p, cmd, board, link_pdf, true)?;
    if pages.is_empty() {
        return Err(bad(cmd, "nothing to place"));
    }
    Ok(pages.swap_remove(0))
}

/// Full placement loader used by `file.place`.
fn load_pages(p: &Value, cmd: &str, board: Option<Rect>, link_pdf: bool) -> Result<Vec<Loaded>> {
    load_pages_mode(p, cmd, board, link_pdf, false)
}

fn load_pages_mode(p: &Value, cmd: &str, board: Option<Rect>, link_pdf: bool, single_only: bool) -> Result<Vec<Loaded>> {
    if let Some(v) = p.get("page").filter(|v| !v.is_null()) {
        v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))?;
    }
    let src = fileio::source(p, cmd)?;
    let name = fileio::file_name(src.name);
    // An Affinity file is recognised by its signature, whatever its name says.
    if fileio::TEXT_EXTS.contains(&fileio::extension(src.name).as_str()) && !vectorcraft_affinity::is_affinity(&src.bytes) {
        let text = text::import(&src.bytes, text::TextOptions::parse(p, cmd)?, cmd)?;
        if text.trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        let art = Art::Text(text::Story::plain(text));
        return Ok(vec![Loaded { name, format: &text::FORMAT, art, natural: TEXT_FRAME, warnings: vec![], link: None, board: None }]);
    }
    // An RTF file is recognized by its content, whatever its name says (a .txt file is plain text).
    if rtf::is_rtf(&src.bytes) {
        let story = rtf::import(&src.bytes, cmd)?;
        if story.plain_text().trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(vec![Loaded {
            name,
            format: &rtf::FORMAT,
            art: Art::Text(story),
            natural: TEXT_FRAME,
            warnings: vec![],
            link: None,
            board: None,
        }]);
    }
    // A Word document is recognized by its name or by its package's `word/document.xml`.
    if docx::is_docx(src.name, &src.bytes) {
        let story = docx::import(&src.bytes, cmd)?;
        if story.plain_text().trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(vec![Loaded {
            name,
            format: &docx::FORMAT,
            art: Art::Text(story),
            natural: TEXT_FRAME,
            warnings: vec![],
            link: None,
            board: None,
        }]);
    }
    // A Word 97-2003 document is recognized by its name or by its compound file's WordDocument
    // stream.
    if doc::is_doc(src.name, &src.bytes) {
        let (story, warnings) = doc::import(&src.bytes, cmd)?;
        if story.plain_text().trim().is_empty() {
            return Err(bad(cmd, format!("`{name}` has no text to place")));
        }
        return Ok(vec![Loaded { name, format: &doc::FORMAT, art: Art::Text(story), natural: TEXT_FRAME, warnings, link: None, board: None }]);
    }
    let format = fileio::detect(src.name, &src.bytes)
        .ok_or_else(|| bad(cmd, format!("can't place `{name}`: not a format VectorCraft reads (see document.formats)")))?;
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
    if !pdf && p.get("pages").is_some_and(|v| !v.is_null()) {
        return Err(bad(cmd, "only PDF files support `pages`"));
    }
    // A DCS file stays linked to its file, which its plates print from; its parameters are checked
    // as an EPS file's are.
    if dcs::is_dcs(&src.bytes) {
        let path = src.path.filter(|_| bool_or(p, "link", true)).map(fileio::absolute_path).ok_or_else(|| bad(cmd, dcs::LINK_ONLY))?;
        let a = dcs::read(&src.bytes, Some(&path)).map_err(|e| bad(cmd, format!("{name}: {e}")))?;
        let link = super::links::link_info(&path, &src.bytes);
        let (natural, warnings) = (a.page, a.warnings.clone());
        return Ok(vec![Loaded { name, format, art: Art::Dcs(Box::new(a)), natural, warnings, link: Some(link), board: None }]);
    }
    // A VectorCraft document: placed linked from its file, else as an editable copy.
    if matches!(format.id, "vectorcraft" | "template") {
        let page = match p.get("page") {
            None => 1,
            Some(v) => v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))? as usize,
        } as u32;
        if let Some(path) = src.path.filter(|_| bool_or(p, "link", true)) {
            let s = document::source(src.name, &src.bytes, Some(path), page, !crop, PageBox::Crop, cmd)?;
            let link = LinkInfo { page: Some(page), ..super::links::link_info(path, &src.bytes) };
            let (natural, warnings) = (s.frame, s.warnings.clone());
            return Ok(vec![Loaded { name, format, art: Art::Document(s, link.clone(), !crop), natural, warnings, link: Some(link), board: None }]);
        }
        let a = document::document_art(src.name, &src.bytes, src.path, page, !crop, PageBox::Crop, cmd)?;
        let art = Art::Vector { src: Box::new(a.doc), nodes: a.nodes, clip: a.clip };
        return Ok(vec![Loaded { name, format, art, natural: a.frame, warnings: a.warnings, link: None, board: None }]);
    }
    if format.raster {
        let img = fileio::raster_image(&src.bytes)?;
        let (sx, sy) = pt_per_px(img.ppi);
        let natural = Rect::new(0.0, 0.0, img.width as f64 * sx, img.height as f64 * sy);
        let link = src.path.map(|path| super::links::link_info(path, &src.bytes));
        let warnings = fileio::size_note(&img.blob.bytes).into_iter().collect();
        return Ok(vec![Loaded { name, format, art: Art::Image(img), natural, warnings, link, board: None }]);
    }
    if pdf {
        let password = opts.password.as_deref();

        // When `single_only` is true and no explicit page/range requires bounds checking against total_pages,
        // avoid enumerating all pages if page 1 is requested directly.
        let total_pages = if single_only
            && p.get("page").is_none_or(Value::is_null)
            && p.get("pages").and_then(Value::as_str).is_none_or(|s| s.trim().eq_ignore_ascii_case("all"))
        {
            None
        } else {
            match vectorcraft_pdf::open_pdf(&src.bytes, password) {
                Ok(opened) => {
                    let count = opened.page_count().max(1);
                    let is_placeholder_ai =
                        count == 1 && fileio::opened_page_document(&src.bytes, &opened, 0, &opts).err().is_some_and(|e| fileio::is_placeholder(&e));
                    if is_placeholder_ai { None } else { Some(count) }
                }
                Err(_) => Some(1),
            }
        };

        let pages_to_place = resolve_pdf_pages(p, cmd, single_only, total_pages)?;
        let mut loaded_pages = Vec::with_capacity(pages_to_place.len());
        let mut total_nodes: usize = 0;

        let linked = link_pdf && bool_or(p, "link", true) && opts.password.is_none() && opts.text_as == vectorcraft_pdf::TextAs::Text;
        let mut shared_bytes: Option<Arc<Vec<u8>>> = None;

        for page_num in pages_to_place {
            if let Some(path) = src.path.filter(|_| linked) {
                let page_u32 = page_num as u32;
                let s = document::source_with_blob(
                    src.name,
                    &src.bytes,
                    &mut shared_bytes,
                    Some(path),
                    page_u32,
                    !crop,
                    document::page_box(opts.crop),
                    cmd,
                )?;
                total_nodes = total_nodes.saturating_add(s.node_count);
                if total_nodes > MAX_PLACE_TOTAL_NODES {
                    return Err(bad(cmd, format!("multipage PDF placement exceeded the {MAX_PLACE_TOTAL_NODES}-node budget on page {page_num}")));
                }
                let link = LinkInfo { page: Some(page_u32), ..super::links::link_info(path, &src.bytes) };
                let (natural, warnings) = (s.frame, s.warnings.clone());
                loaded_pages.push(Loaded {
                    name: name.clone(),
                    format,
                    art: Art::Document(s, link.clone(), !crop),
                    natural,
                    warnings,
                    link: Some(link),
                    board: None,
                });
            } else {
                let a = pdf_art(src.name, &src.bytes, src.path, page_num, opts.clone(), !crop, cmd)?;
                let page_nodes = a.nodes.iter().map(|n| n.count()).sum::<usize>();
                total_nodes = total_nodes.saturating_add(page_nodes);
                if total_nodes > MAX_PLACE_TOTAL_NODES {
                    return Err(bad(cmd, format!("multipage PDF placement exceeded the {MAX_PLACE_TOTAL_NODES}-node budget on page {page_num}")));
                }
                let art = Art::Vector { src: Box::new(a.doc), nodes: a.nodes, clip: a.clip };
                loaded_pages.push(Loaded { name: name.clone(), format, art, natural: a.frame, warnings: a.warnings, link: None, board: None });
            }
        }
        return Ok(loaded_pages);
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
    let page = match p.get("page") {
        None => 1,
        Some(v) => v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))? as usize,
    };
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
    Ok(vec![Loaded { name, format, art: Art::Vector { src: Box::new(doc), nodes, clip }, natural, warnings, link: None, board }])
}

/// Page `page` (1-based) of the PDF/.ai/.ait file `src` (`bytes`, read from `path` when there is
/// one) as art to place: clipped to the box `opts.crop` names, or (`bounding`) the bounds of its
/// art; read with `opts.password`. Only the page placed is read. A .ai saved without PDF
/// compatibility is read from its editing data, as it opens, and placed as an EPS is: the art on
/// artboard `page`, its linked images found from its folder. So is page 1 of a .ai or PDF whose
/// PDF part can't be read.
pub(crate) fn pdf_art(
    src: &str,
    bytes: &[u8],
    path: Option<&str>,
    page: usize,
    opts: fileio::LoadOptions,
    bounding: bool,
    cmd: &str,
) -> Result<document::DocumentArt> {
    pdf_art_opened(src, bytes, None, path, page, opts, bounding, cmd)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn pdf_art_opened(
    src: &str,
    bytes: &[u8],
    opened: Option<&vectorcraft_pdf::Opened>,
    path: Option<&str>,
    page: usize,
    opts: fileio::LoadOptions,
    bounding: bool,
    cmd: &str,
) -> Result<document::DocumentArt> {
    let name = fileio::file_name(src);
    let index = page.checked_sub(1).ok_or_else(|| bad(cmd, "page must be a whole number from 1"))?;
    let o = fileio::LoadOptions { crop: if bounding { CropTo::Crop } else { opts.crop }, ..opts };
    let mut read =
        if let Some(opened) = opened { fileio::opened_page_document(bytes, opened, index, &o) } else { fileio::page_document(bytes, index, &o) };
    // Such a file can have more artboards than placeholder pages.
    if page > 1
        && read.is_err()
        && let Err(e) =
            if let Some(opened) = opened { fileio::opened_page_document(bytes, opened, 0, &o) } else { fileio::page_document(bytes, 0, &o) }
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
    Ok(document::DocumentArt { doc, nodes, clip, frame, warnings })
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
        Art::Image(img) => {
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
            Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)))
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
/// `image_xf`); other objects keep their centre.
fn replace_xf(d: &Document, old: &Node, natural: Rect, image_xf: bool) -> Affine {
    if image_xf && let NodeKind::Image(im) = &old.kind {
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

/// `rect` fitted with `natural`'s aspect, centred.
fn fit(natural: Rect, r: Rect) -> Affine {
    let ratio = |a: f64, b: f64| (b > 1e-9).then(|| a / b);
    let k = [ratio(r.width(), natural.width()), ratio(r.height(), natural.height())].into_iter().flatten().reduce(f64::min).unwrap_or(1.0);
    Affine::translate(r.center().to_vec2()) * Affine::scale(k) * Affine::translate(-natural.center().to_vec2())
}

/// What `file.place` takes from the session before it reads the file: the params, the object
/// Replace swaps and the artboard under `at`, which a DXF drawing is laid out on
/// ([`Session::prepare_place`]). [`PlaceJob::read`] reads and decodes the file without the session,
/// on any thread.
pub struct PlaceJob {
    params: Value,
    old: Option<NodeId>,
    /// The object Replace swaps as it was when Place started.
    replaced: Option<Node>,
    target: Option<Rect>,
}

/// A file [`PlaceJob::read`] read, which [`Session::place_read`] places.
pub struct PlaceRead {
    job: PlaceJob,
    loaded: Vec<Loaded>,
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
        let mut loaded = load_pages(&self.params, PLACE, self.target, true)?;
        for l in &mut loaded {
            let linked = l.link.is_some() && bool_or(&self.params, "link", true);
            match &mut l.art {
                Art::Image(img) => {
                    let unread = super::links::kept_unread(&img.blob.bytes);
                    img.blob = super::links::readable_and_kept(img.blob.clone(), unread, linked)?;
                }
                Art::Dcs(a) => a.img.blob = super::links::readable(a.img.blob.clone(), false, true)?,
                _ => {}
            }
        }
        Ok(PlaceRead { job: self, loaded })
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
    /// reply. The object Replace swaps must still be in the document as it was when Place started;
    /// when the selection changed meanwhile, `select.set` selects that object again first, so a
    /// replay of the journal replaces the same object.
    pub fn place_read(&mut self, read: PlaceRead) -> Result<Value> {
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
    let has_page = p.get("page").is_some_and(|v| !v.is_null());
    let has_pages = p.get("pages").is_some_and(|v| !v.is_null());
    if has_page && has_pages {
        return Err(bad(PLACE, "specify either page or pages, not both"));
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
    Ok(PlaceJob { params: p.clone(), old, replaced, target: board_at(&st.doc, at) })
}

/// Place the file `read` holds in the active document.
fn apply(s: &mut Session, read: PlaceRead) -> Result<Value> {
    let PlaceRead { job: PlaceJob { params, old, target: read_for, .. }, mut loaded } = read;
    let p = &params;
    let template = bool_or(p, "template", false);
    let at = at_param(p)?;
    let rect = rect_param(p)?;
    let st = s.doc()?;
    let old_node = match old {
        Some(id) => Some(st.doc.node(id).ok_or_else(|| bad(PLACE, "replace: the object to replace was changed or deleted"))?),
        None => None,
    };
    let target = board_at(&st.doc, at);

    if loaded.len() == 1 {
        let mut loaded = loaded.remove(0);
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
        let m = match (old_node, rect) {
            (Some(o), _) => replace_xf(&st.doc, o, natural, !is_text),
            (None, Some(r)) => fit(natural, r),
            // Its artboard's bottom-left corner on the target's.
            (None, None) if let (Some(src), Some(to)) = (loaded.board, target) => Affine::translate((to.x0 - src.x0, to.y1 - src.y1)),
            (None, None) => {
                let c = at.or_else(|| st.doc.artboards.first().map(|a| a.rect.center())).unwrap_or_default();
                Affine::translate(c - natural.center())
            }
        };
        let parent = st.insertion_parent();
        let below = st.current_layer().or_else(|| parent.and_then(|p| st.doc.layer_of(p)));
        let link = loaded.link.take().filter(|_| bool_or(p, "link", true));
        let linked = link.is_some();
        let (name, format, mut warnings) = (loaded.name.clone(), loaded.format.id, std::mem::take(&mut loaded.warnings));
        let dcs_clipped = matches!(&loaded.art, Art::Dcs(a) if a.clip.is_some());
        if let Art::Dcs(a) = &loaded.art {
            warnings.extend(dcs::swatch_notes(&st.doc, &a.spots));
        }
        let (id, size) = s.edit("Place", |d, sel| {
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
            // Template art is locked: nothing to select.
            if template {
                sel.clear()
            } else {
                sel.set([id])
            }
            Ok((id, size))
        })?;
        let mut out = json!({ "ids": [id.0], "name": name, "format": format, "linked": linked, "width": size.width(), "height": size.height(), "warnings": warnings });
        // A DCS file clipped by its clipping path: its image, which the Links commands take.
        if dcs_clipped {
            let image = s.doc()?.doc.node(id).and_then(|n| match &n.kind {
                NodeKind::Group { children, clip: true } => children.get(1).filter(|c| matches!(c.kind, NodeKind::Image(_))).map(|c| c.id),
                _ => None,
            });
            if let Some(image) = image {
                out["image"] = json!(image.0);
            }
        }
        return Ok(out);
    }

    // Multi-page placement
    let first = loaded.first().ok_or_else(|| bad(PLACE, "nothing to place"))?;
    let name = first.name.clone();
    let format = first.format.id;

    if at.is_some() && rect.is_some() {
        return Err(bad(PLACE, "specify either at or rect, not both"));
    }
    if old.is_some() && (at.is_some() || rect.is_some()) {
        return Err(bad(PLACE, "replace cannot be combined with at or rect"));
    }

    let page_gap = 20.0_f64;
    let mut local_offsets = Vec::with_capacity(loaded.len());
    let mut layout_w = 0.0_f64;
    let mut max_h = 0.0_f64;

    for (idx, item) in loaded.iter().enumerate() {
        if idx > 0 {
            layout_w += page_gap;
        }
        local_offsets.push(layout_w);
        let page_bounds = item.natural;
        let page_width = page_bounds.width().max(1.0);
        let page_height = page_bounds.height().max(1.0);
        layout_w += page_width;
        if page_height > max_h {
            max_h = page_height;
        }
    }

    let layout_bounds = Rect::new(0.0, 0.0, layout_w.max(1.0), max_h.max(1.0));
    if !layout_bounds.width().is_finite() || !layout_bounds.height().is_finite() {
        return Err(bad(PLACE, "multipage layout bounds are not finite"));
    }

    let layout_xf = match (old_node, rect) {
        (Some(o), _) => replace_xf(&st.doc, o, layout_bounds, true),
        (None, Some(r)) => fit(layout_bounds, r),
        (None, None) => {
            let c = at.or_else(|| st.doc.artboards.first().map(|a| a.rect.center())).unwrap_or_default();
            Affine::translate(c - layout_bounds.center())
        }
    };

    // Transform all four corners of layout_bounds and take their axis-aligned bounding box
    // so rotated replacements (e.g. 90 degrees) report swapped width and height.
    let corners = [
        layout_xf * Point::new(layout_bounds.x0, layout_bounds.y0),
        layout_xf * Point::new(layout_bounds.x1, layout_bounds.y0),
        layout_xf * Point::new(layout_bounds.x1, layout_bounds.y1),
        layout_xf * Point::new(layout_bounds.x0, layout_bounds.y1),
    ];
    let min_x = corners.iter().map(|pt| pt.x).fold(f64::INFINITY, f64::min);
    let max_x = corners.iter().map(|pt| pt.x).fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners.iter().map(|pt| pt.y).fold(f64::INFINITY, f64::min);
    let max_y = corners.iter().map(|pt| pt.y).fold(f64::NEG_INFINITY, f64::max);
    let width = (max_x - min_x).max(0.0);
    let height = (max_y - min_y).max(0.0);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(bad(PLACE, "replace: invalid replacement geometry"));
    }

    let parent = st.insertion_parent();
    let below = st.current_layer().or_else(|| parent.and_then(|p| st.doc.layer_of(p)));

    let (ids, any_linked, warnings) = s.edit("Place", |d, sel| {
        let (into, mut index) = match old {
            Some(old) => {
                let (par, idx, _) = d.position(old).ok_or(EngineError::NoNode(old))?;
                d.remove(old)?;
                (par, idx)
            }
            None if template => (Some(template_layer(d, below, &name)?), usize::MAX),
            None => (parent, usize::MAX),
        };
        let mut ids = Vec::with_capacity(loaded.len());
        let mut any_linked = false;
        let mut warnings = Vec::new();
        for (mut item, local_x) in loaded.into_iter().zip(local_offsets) {
            let page_bounds = item.natural;
            warnings.append(&mut item.warnings);
            let link = item.link.take().filter(|_| bool_or(p, "link", true));
            if link.is_some() {
                any_linked = true;
            }
            let local_m = Affine::translate((local_x - page_bounds.x0, -page_bounds.y0));
            let m = layout_xf * local_m;
            let node = transformed(build(d, item, link), m);
            let id = d.insert(into, index, node)?;
            if index != usize::MAX {
                index += 1;
            }
            ids.push(id);
        }
        if template {
            sel.clear();
        } else {
            sel.set(ids.iter().copied());
        }
        Ok((ids, any_linked, warnings))
    })?;

    let id_vals: Vec<Value> = ids.into_iter().map(|id| json!(id.0)).collect();
    Ok(json!({ "ids": id_vals, "name": name, "format": format, "linked": any_linked, "width": width, "height": height, "warnings": warnings }))
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
    params: Value,
    files: Vec<Value>,
}

/// The files [`QueueJob::read`] read, which [`Session::place_queue_read`] loads the place cursor
/// with.
pub struct QueueRead {
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
        read_queue(self.params, self.files)
    }
}

/// Read each of `queued`, the files of `file.place.queue` with params `p`, for its size and
/// thumbnail.
fn read_queue(p: Value, queued: Vec<Value>) -> Result<QueueRead> {
    let thumbnail = p.get("thumbnail").and_then(Value::as_f64);
    let (mut entries, mut files, mut skipped) = (vec![], vec![], vec![]);
    for mut q in queued {
        for k in ["link", "template", "page", "crop", "text", "dxf"] {
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
                Art::Image(img) => {
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
                skipped.push(json!({ "name": name, "error": e.to_string() }));
            }
        }
    }
    Ok(QueueRead { params: p, entries, files, skipped })
}

impl Session {
    /// `file.place.queue`'s checks (with no document open, the error `file.place.queue` gives) of
    /// `params` → what reads the files ([`QueueJob::read`]; on the
    /// desktop the Place dialog's OK with several files does this off the UI thread).
    pub fn prepare_place_queue(&self, params: &Value) -> Result<QueueJob> {
        has_doc(self).map_err(|why| EngineError::Disabled(QUEUE.into(), why))?;
        check_switches(params, &["link", "template"], QUEUE)?;
        check_numbers(params, &["thumbnail"], QUEUE)?;
        Ok(QueueJob { params: params.clone(), files: queued_files(params)? })
    }

    /// Run `file.place.queue` with the params of `read` as a command that loads the place cursor
    /// with the files `read` holds instead of reading them again → `file.place.queue`'s reply.
    pub fn place_queue_read(&mut self, read: QueueRead) -> Result<Value> {
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
    use image::{ExtendedColorType as E, ImageDecoder as _};
    let kind = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .map(|d| d.original_color_type());
    match kind {
        Some(E::L1 | E::La1 | E::L2 | E::La2 | E::L4 | E::La4 | E::L8 | E::La8 | E::L16 | E::La16) => "Grayscale",
        Some(E::Cmyk8 | E::Cmyk16) => "CMYK",
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
    let mut l = load(p, "file.place.info", None, false)?;
    let natural = l.natural;
    let mut out = json!({
        "name": l.name,
        "format": l.format.id,
        "width": natural.width(),
        "height": natural.height(),
        "warnings": std::mem::take(&mut l.warnings),
    });
    if let Art::Image(img) = &l.art {
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

pub(crate) fn image_info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "image.info";
    let st = s.doc()?;
    let id = match (object_param(p, "id", C)?, &st.selection.objects[..]) {
        (Some(id), _) => id,
        (None, [id]) => *id,
        _ => return Err(bad(C, "select one image or pass id")),
    };
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    let NodeKind::Image(im) = &n.kind else { return Err(bad(C, format!("object {} is not an image", id.0))) };
    // Effective resolution: 72 pt per inch over the points one pixel spans along each axis.
    let [a, b, c, d, ..] = im.xf.as_coeffs();
    let ppi = |x: f64, y: f64| Some(x.hypot(y)).filter(|l| *l > 1e-12).map_or(0.0, |l| 72.0 / l);
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
        "ppi": [ppi(a, b), ppi(c, d)],
        "width": size.width(),
        "height": size.height(),
    }))
}
