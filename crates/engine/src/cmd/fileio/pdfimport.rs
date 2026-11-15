//! Opening and placing PDF (and PDF-compatible `.ai`/`.ait`) files: the options `document.open`
//! and `file.place` read ([`LoadOptions`]), the pages they import and `document.pdfInfo` (pages,
//! boxes, a thumbnail; one page for a PostScript file) for the Import PDF dialog.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Artboard, ColorMode, Document, LayerColor, LinkInfo, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_pdf::{Choice, CropTo, ImportOptions, PdfError, TextAs};

use super::super::colormgmt::Grays;
use super::super::place::document;
use super::super::{Result, Session, bad, check_numbers, number_or, parse_range, text_param};
use super::{err, source};

/// The `document.open` options besides the file. The PDF ones (pages, box, password, text,
/// layers) and the DXF ones are ignored by other formats.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadOptions {
    /// 1-based pages to import, such as `"2-3, 5"`; `None` = every page.
    pub pages: Option<String>,
    /// The box each page's artboard (or placed frame) gets.
    pub crop: CropTo,
    /// The password of an encrypted PDF.
    pub password: Option<String>,
    /// The colour mode the document opens in, its colours converted as Document Color Mode does
    /// (default: the file's; a PDF painted mostly in CMYK opens in CMYK).
    pub color_mode: Option<ColorMode>,
    /// How that conversion separates RGB greys (`grays`).
    pub grays: Grays,
    /// What a PDF's text becomes.
    pub text_as: TextAs,
    /// A PDF's optional content groups become layers (else one layer per page, without the art
    /// that is off), and a PSD or PSB file's layers become layers (else its merged image opens).
    pub layers: bool,
    /// A PSD or PSB file's hidden layers come in, hidden (`hiddenLayers`; else they are left out).
    pub hidden_layers: bool,
    /// A PSD or PSB file's slices become user slices (`importSlices`; else they are left out).
    pub import_slices: bool,
    /// How a DXF drawing comes in (the `dxf` param, see [`super::dxfimport`]).
    pub dxf: vectorcraft_cad::ImportOptions,
    /// An Illustrator EPS or `.ai` that carries the editing copy of its art opens from it (its
    /// layers, hidden objects and the art outside its artboards); else only what its page or PDF
    /// part prints comes in.
    pub editing_data: bool,
    /// A `.ai` opens from its editing copy even where that draws differently from its PDF part
    /// (which can be older than it), with a warning, instead of from the PDF part.
    pub trust_editing_data: bool,
    /// The encoding names in a `.ai` or EPS's editing data are read in when they aren't UTF-8
    /// (`legacyNameEncoding`).
    pub legacy_names: vectorcraft_eps::NameEncoding,
    /// A PDF or `.ai` saved with Preserve Editing opens from the VectorCraft document it carries;
    /// off, it opens as another app would open it, from the editing copy of its art or its pages.
    pub native: bool,
    /// Each page of a PDF opens as a placed document linked to that page of its file, on an
    /// artboard of its own, instead of as editable art (`linkPages`; see [`linked_pages`]).
    pub link_pages: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            pages: None,
            crop: CropTo::default(),
            password: None,
            color_mode: None,
            grays: Grays::default(),
            text_as: TextAs::default(),
            layers: true,
            hidden_layers: false,
            import_slices: true,
            dxf: Default::default(),
            editing_data: true,
            trust_editing_data: false,
            legacy_names: vectorcraft_eps::NameEncoding::default(),
            native: true,
            link_pages: false,
        }
    }
}

impl LoadOptions {
    /// From command params: `pages` (`"2-3, 5"`, a page number or `"all"`), `page` (one page, when
    /// `pages` isn't given), `cropTo` (or `crop`), `password`, `colorMode` (`rgb` | `cmyk`) with `grays`,
    /// `textAs` (`text` | `outlines`), `layers` (true | false), `hiddenLayers` (true | false),
    /// `importSlices` (true | false), `editingData` (true | false | `always`),
    /// `legacyNameEncoding` (one of [`vectorcraft_eps::NameEncoding::LABELS`]), `linkPages` (true |
    /// false) and `dxf` (the DXF import options).
    pub fn from_params(cmd: &str, p: &Value) -> Result<Self> {
        let pages = match p.get("pages").filter(|v| !v.is_null()).or_else(|| p.get("page").filter(|v| !v.is_null())) {
            None => None,
            Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("all") => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            Some(v) => return Err(bad(cmd, format!("pages must be a range such as \"2-3\" or a page number, not {v}"))),
        };
        let (crop_to, crop_alias) = (text_param(p, "cropTo", cmd)?, text_param(p, "crop", cmd)?);
        let crop = choice(cmd, "cropTo", crop_to.or(crop_alias))?;
        let password = text_param(p, "password", cmd)?.filter(|s| !s.is_empty()).map(str::to_string);
        let color_mode = match text_param(p, "colorMode", cmd)?.map(str::to_ascii_lowercase).as_deref() {
            None => None,
            Some("rgb") => Some(ColorMode::Rgb),
            Some("cmyk") => Some(ColorMode::Cmyk),
            Some(m) => return Err(bad(cmd, format!("colorMode must be rgb or cmyk, not `{m}`"))),
        };
        let grays = Grays::param(cmd, p)?;
        let text_as = choice(cmd, "textAs", text_param(p, "textAs", cmd)?)?;
        let layers = match p.get("layers").filter(|v| !v.is_null()) {
            None => true,
            Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "layers must be true or false"))?,
        };
        let hidden_layers = match p.get("hiddenLayers").filter(|v| !v.is_null()) {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "hiddenLayers must be true or false"))?,
        };
        let import_slices = match p.get("importSlices").filter(|v| !v.is_null()) {
            None => true,
            Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "importSlices must be true or false"))?,
        };
        let (editing_data, trust_editing_data) = match p.get("editingData").filter(|v| !v.is_null()) {
            None => (true, false),
            Some(Value::String(s)) if s.eq_ignore_ascii_case("always") => (true, true),
            Some(v) => (v.as_bool().ok_or_else(|| bad(cmd, "editingData must be true, false or \"always\""))?, false),
        };
        let legacy_names = match p.get("legacyNameEncoding").filter(|v| !v.is_null()) {
            None => vectorcraft_eps::NameEncoding::default(),
            Some(v) => v
                .as_str()
                .and_then(vectorcraft_eps::NameEncoding::from_label)
                .ok_or_else(|| bad(cmd, format!("legacyNameEncoding must be one of {}", vectorcraft_eps::NameEncoding::LABELS.join(", "))))?,
        };
        let link_pages = match p.get("linkPages").filter(|v| !v.is_null()) {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| bad(cmd, "linkPages must be true or false"))?,
        };
        let dxf = super::dxfimport::options(cmd, p.get("dxf"))?;
        Ok(Self {
            pages,
            crop,
            password,
            color_mode,
            grays,
            text_as,
            layers,
            hidden_layers,
            import_slices,
            dxf,
            editing_data,
            trust_editing_data,
            legacy_names,
            native: true,
            link_pages,
        })
    }

    /// Does the document read only part of the file, or read it differently (a page range, another
    /// box, a password, without its hidden layers, without the VectorCraft document it carries, its
    /// pages linked)? Writing it back would lose the rest, so Save doesn't (it asks for a name).
    pub fn is_partial(&self) -> bool {
        self.pages.is_some() || self.crop != CropTo::default() || self.password.is_some() || !self.layers || !self.native || self.link_pages
    }

    /// The PDF import options: `pages` resolved against the file's page count. A PDF-compatible
    /// `.ai`, or a PDF saved with its editing data, asked for its crop box opens on its trim boxes,
    /// its editor's artboards, with the bleed round them (its crop box takes the bleed in).
    fn import_options(&self, opened: &vectorcraft_pdf::Opened, editor_document: bool) -> Result<ImportOptions> {
        let pages = match &self.pages {
            None => None,
            Some(range) => Some(parse_range(range, opened.page_count()).map_err(|e| err(format!("pages: {e}")))?),
        };
        let is_ai = self.crop == CropTo::Crop && opened.has_illustrator_data();
        Ok(ImportOptions {
            pages,
            crop: if is_ai { CropTo::Trim } else { self.crop },
            password: self.password.clone(),
            text_as: self.text_as,
            layers: self.layers,
            uncalibrated_rgb: editor_document,
            ..Default::default()
        })
    }
}

/// The option of `T` named `id` (any case), for param `key`; not given: the default.
fn choice<T: Choice + Default>(cmd: &str, key: &str, id: Option<&str>) -> Result<T> {
    let Some(id) = id else { return Ok(T::default()) };
    T::ALL
        .iter()
        .zip(T::IDS)
        .find(|(_, i)| i.eq_ignore_ascii_case(id))
        .map(|(t, _)| *t)
        .ok_or_else(|| bad(cmd, format!("{key} must be one of {}", T::IDS.join(", "))))
}

/// Import the pages `o` pick of `opened`, the PDF `bytes` already read
/// ([`vectorcraft_pdf::open_pdf`]) → the document, import notes and what else the import tells.
/// `editor_document`: a `.ai` file.
pub(super) fn import(
    bytes: &[u8],
    opened: &vectorcraft_pdf::Opened,
    o: &LoadOptions,
    editor_document: bool,
) -> Result<vectorcraft_pdf::ImportReport> {
    let options = o.import_options(opened, editor_document)?;
    vectorcraft_pdf::import_opened(bytes, opened, &options).map_err(err)
}

/// Page `page` (0-based) of a PDF as a document of one artboard (its `o.crop` box) and one
/// layer of what it shows, with the same text choice as Open, and the import notes.
pub fn page_document(bytes: &[u8], page: usize, o: &LoadOptions) -> Result<(Document, Vec<String>)> {
    vectorcraft_pdf::import_with_report(bytes, &page_options(page, o)).map(|r| (r.document, r.warnings)).map_err(err)
}

/// [`page_document`] of `opened`, the PDF `bytes` already read.
pub(crate) fn opened_page_document(bytes: &[u8], opened: &vectorcraft_pdf::Opened, page: usize, o: &LoadOptions) -> Result<(Document, Vec<String>)> {
    vectorcraft_pdf::import_opened(bytes, opened, &page_options(page, o)).map(|r| (r.document, r.warnings)).map_err(err)
}

/// The import options of [`page_document`].
fn page_options(page: usize, o: &LoadOptions) -> ImportOptions {
    ImportOptions { pages: Some(vec![page]), crop: o.crop, password: o.password.clone(), text_as: o.text_as, layers: false, ..Default::default() }
}

/// `document.open` with `linkPages`: the pages `o` picks of the PDF (or PDF-compatible `.ai` or
/// `.ait`) `name`, read from `path`, as a document holding one placed document per page, linked to
/// that page of the file as File › Place with Link places one ([`document::source`]). Each page
/// gets an artboard of its `o.crop` box (its art's bounds for `bounding`, a blank page's crop box;
/// a PDF-compatible `.ai` asked for its crop box gets its trim box, as it opens), in a row from the
/// left with the gap a PDF's artboards open with, and a layer named after the page. On its trim box,
/// a page shows its bleed box round the artboard, and the document's bleed is the pages' bleed (the
/// widest on each side); the artboards are then the gap apart plus the bleed on both sides. The document opens
/// in CMYK when most of the pages read in CMYK, unless `o.color_mode` names a mode. Refused without a
/// path, for any other file, with a password (a linked page is read again without one), with
/// `textAs: outlines` (a linked page shows the file's own type) and for a file over
/// [`document::MAX_FILE`] → the document and the notes of reading its pages.
pub(super) fn linked_pages(name: &str, bytes: &[u8], path: Option<&str>, o: &LoadOptions) -> Result<(Document, Vec<String>)> {
    const C: &str = "document.open";
    let file = super::file_name(name);
    if !document::is_pdf(name, bytes) {
        return Err(bad(C, format!("linkPages: `{file}` isn't a PDF or a PDF-compatible .ai file")));
    }
    let path = path.ok_or_else(|| bad(C, "linkPages needs the file's path: linked pages are read from it"))?;
    if o.password.is_some() {
        return Err(bad(C, "linkPages can't open a file with a password: linked pages are read again without it"));
    }
    if o.text_as != TextAs::Text {
        return Err(bad(C, "linkPages can't be used with textAs: outlines: linked pages show the file's own type"));
    }
    if bytes.len() > document::MAX_FILE {
        return Err(bad(C, format!("linkPages: `{file}` is over {} MB", document::MAX_FILE >> 20)));
    }
    // The file is read once: every page is read from it.
    let opened = vectorcraft_pdf::open_pdf(bytes, None).map_err(err)?;
    let count = opened.page_count();
    let picked = match &o.pages {
        None => (0..count).collect(),
        Some(range) => parse_range(range, count).map_err(|e| err(format!("pages: {e}")))?,
    };
    let bounding = o.crop == CropTo::Bounding;
    // A PDF-compatible `.ai` asked for its crop box opens on its trim boxes ([`LoadOptions::import_options`]).
    let crop = if o.crop == CropTo::Crop && opened.has_illustrator_data() { CropTo::Trim } else { o.crop };
    let mut doc = Document::new(1.0, 1.0);
    let board = doc.artboards.first().cloned().ok_or_else(|| err("a new document has no artboard"))?;
    doc.artboards.clear();
    doc.layers.clear();
    let link = crate::cmd::links::link_info(path, bytes);
    let gap = ImportOptions::default().artboard_gap;
    // Every page's object keeps the same one copy of the file.
    let shared = Arc::new(bytes.to_vec());
    // The file is hashed once for all its pages.
    let hash = link.hash.clone().unwrap_or_else(|| vectorcraft_doc::links::hash_bytes(bytes));
    let (mut warnings, mut cmyk_pages) = (Vec::<String>::new(), 0);
    // Each page as read, with its bleed (top, bottom, left, right). The pages are laid out once
    // the document's bleed is known.
    let mut read_pages = Vec::with_capacity(picked.len());
    for &index in &picked {
        let page = index as u32 + 1;
        let file_once = document::SharedFile { bytes: &shared, hash: &hash, opened: &opened };
        // On its trim box, a page shows its bleed box, the artboard on the trim box inside it, so
        // the art in the bleed shows and prints as bleed.
        let bleed = if crop == CropTo::Trim { opened.page_bleed(index) } else { None };
        let page_box = if bleed.is_some() { vectorcraft_doc::PageBox::Bleed } else { document::page_box(crop) };
        let read = |bounding| document::shared_source(name, file_once, Some(path), page, bounding, page_box, C);
        // A blank page has no art's bounds: it keeps its crop box, as opening it as art does.
        let (mut s, bounding) = match read(bounding) {
            Err(e) if bounding && e.to_string().contains("has no art to place") => (read(false)?, false),
            s => (s?, bounding),
        };
        // A page that is only a placeholder (a `.ai` saved without PDF compatibility) shows the
        // file's editing data, not a page of the file.
        if s.editing_data {
            return Err(bad(C, format!("linkPages: `{file}` isn't a PDF or a PDF-compatible .ai file")));
        }
        cmyk_pages += usize::from(s.color_mode == ColorMode::Cmyk);
        // The document's bleed is the widest of the pages' on each side.
        let sides = bleed.unwrap_or_default();
        for (side, b) in doc.setup.bleed.iter_mut().zip(sides) {
            *side = side.max(b.min(vectorcraft_doc::setup::MAX_BLEED));
        }
        for w in std::mem::take(&mut s.warnings) {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        read_pages.push((page, s, bounding, sides));
    }
    // Each artboard is the gap from the one before plus the bleed on both sides of it: the
    // document's bleed, which export adds round every artboard, or a page's own bleed box where it
    // reaches farther. Neither page then shows or exports the other's art.
    let [_, _, doc_left, doc_right] = doc.setup.bleed;
    let mut x = 0.0;
    for (i, (page, s, bounding, [top, bottom, left, right])) in read_pages.into_iter().enumerate() {
        if i > 0 {
            x += doc_left.max(left);
        }
        let mut node = document::node(&mut doc, &s, LinkInfo { page: Some(page), ..link.clone() }, bounding);
        if let NodeKind::PlacedDocument(p) = &mut node.kind {
            p.xf = Affine::translate((x - left, -top));
        }
        node.name = Some(file.clone());
        let mut layer = Node::layer(doc.alloc_id(), &format!("Page {page}"), LayerColor::Preset((i % 27) as u8));
        if let NodeKind::Layer { children, .. } = &mut layer.kind {
            children.push(Arc::new(node));
        }
        doc.layers.push(Arc::new(layer));
        let rect = Rect::new(x, 0.0, x + s.frame.width() - left - right, s.frame.height() - top - bottom);
        doc.artboards.push(Artboard { id: i as u32 + 1, name: format!("Artboard {}", i + 1), rect, ..board.clone() });
        x = rect.x1 + doc_right.max(right) + gap;
    }
    // Pages painted mostly in CMYK read as CMYK documents, as opening them as art does: the
    // document opens in CMYK (with the CMYK default swatches) when most of its pages do.
    if cmyk_pages * 2 > picked.len() {
        doc.color_mode = ColorMode::Cmyk;
        let (swatches, groups) = vectorcraft_color::default_swatches(ColorMode::Cmyk.model());
        (doc.swatches, doc.swatch_groups) = (swatches.into(), groups.into());
    }
    if let Some(mode) = o.color_mode.filter(|m| *m != doc.color_mode) {
        super::super::colormgmt::set_color_mode(&mut doc, mode, true, None, o.grays);
    }
    Ok((doc, warnings))
}

const INFO: &str = "document.pdfInfo";

/// `document.pdfInfo`.
pub(super) fn pdf_info(_: &mut Session, p: &Value) -> Result<Value> {
    let bytes = source(p, INFO)?.bytes;
    let o = LoadOptions::from_params(INFO, p)?;
    check_numbers(p, &["thumbnailSize"], INFO)?;
    // Programs whose header is only `%!` are PostScript too, as `clipboard.importEps` reads them.
    if bytes.starts_with(b"%!") || vectorcraft_pdf::is_postscript(&bytes) {
        return postscript_info(&bytes, p);
    }
    // The file is read once, for its pages and the thumbnail.
    let opened = match vectorcraft_pdf::open_pdf(&bytes, o.password.as_deref()) {
        Ok(opened) => opened,
        Err(e @ (PdfError::NeedsPassword | PdfError::WrongPassword)) => {
            return Ok(json!({"pages": 0, "needsPassword": true, "wrongPassword": e == PdfError::WrongPassword, "pageInfo": []}));
        }
        Err(e) => return Err(err(e)),
    };
    let info = opened.info();
    let r4 = |r: vectorcraft_geom::Rect| json!([r.x0, r.y0, r.x1, r.y1]);
    let pages: Vec<Value> = info
        .pages
        .iter()
        .map(|pg| {
            let boxes: serde_json::Map<String, Value> = pg.boxes.iter().map(|(c, r)| (c.id().to_string(), r4(*r))).collect();
            json!({"width": pg.width, "height": pg.height, "rotation": pg.rotation, "boxes": boxes})
        })
        .collect();
    let mut out = json!({"pages": pages.len(), "needsPassword": false, "pageInfo": pages});
    if let Some(page) = thumbnail_page(p, info.pages.len(), "PDF")? {
        let (doc, _) = opened_page_document(&bytes, &opened, page, &o)?;
        out["thumbnail"] = json!(thumbnail(&doc, p)?);
    }
    Ok(out)
}

/// The page (0-based) `document.pdfInfo`'s `thumbnail` names, from 1 to `count`; `None` when it is
/// absent or null. Any other value (0, a negative or fractional number, a string) is an error that
/// names the pages the `what` has.
fn thumbnail_page(p: &Value, count: usize, what: &str) -> Result<Option<usize>> {
    let Some(v) = p.get("thumbnail").filter(|v| !v.is_null()) else { return Ok(None) };
    let page = v.as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1)).filter(|i| *i < count);
    page.map(Some).ok_or_else(|| bad(INFO, format!("thumbnail: no page {v} (the {what} has {count})")))
}

/// `document.pdfInfo` of a PostScript file (an EPS, a `.ps` file, a `.ai` in PostScript form): one
/// page, the page [`vectorcraft_eps::page`] reads, with no PDF boxes. Its thumbnail is the file as
/// the EPS reader reads it without any editing data it carries: what the page prints, or the
/// file's preview when its PostScript can't be read.
fn postscript_info(bytes: &[u8], p: &Value) -> Result<Value> {
    let page = vectorcraft_eps::page(bytes).map_err(err)?;
    let info = json!({"width": page.width(), "height": page.height(), "rotation": 0, "boxes": {}});
    let mut out = json!({"pages": 1, "needsPassword": false, "pageInfo": [info]});
    if thumbnail_page(p, 1, "file")?.is_some() {
        let doc = vectorcraft_eps::import_with(bytes, false).map_err(err)?.document;
        out["thumbnail"] = json!(thumbnail(&doc, p)?);
    }
    Ok(out)
}

/// A PNG of `doc`'s first artboard, `thumbnailSize` px (default 160, from 16 to 1024) on its longer
/// side, base64.
fn thumbnail(doc: &Document, p: &Value) -> Result<String> {
    let size = number_or(p, "thumbnailSize", 160.0, INFO)?.clamp(16.0, 1024.0);
    let r = doc.artboards.first().map(|a| a.rect).ok_or_else(|| err("the page has no area"))?;
    let png = super::encode(doc, "png", &json!({"scale": size / r.width().max(r.height()).max(1.0)}))?;
    Ok(vectorcraft_format::base64_encode(&png))
}
