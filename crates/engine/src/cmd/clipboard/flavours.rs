//! Clipboard flavours: what Copy offers other apps besides SVG (PNG, PDF and plain text) and what
//! Paste takes from them (bitmaps, plain text, PDF, EPS and Windows metafiles).
//!
//! The UI owns the platform clipboard (`Services::system_clipboard` in the egui frontend): on Copy
//! it publishes [`Session::clipboard_flavours`]; before a Paste it turns what another app put on
//! the system clipboard into the internal clipboard with the `clipboard.import*` commands. These
//! commands only convert, so agents can drive them headless.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, ImageObject, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Point};

use super::super::fileio;
use super::super::place::pt_per_px;
use super::*;

/// Plain text: a type-only copy's text, else the SVG markup (Include SVG Code).
pub const TEXT: &str = "text/plain";
/// SVG markup (Include SVG Code).
pub const SVG: &str = "image/svg+xml";
/// A PDF of the copied objects (On Copy: PDF).
pub const PDF: &str = "application/pdf";
/// A PNG of the copied objects (always offered).
pub const PNG: &str = "image/png";
/// Any bitmap (PNG, BMP…), in what Paste asks the system clipboard for.
pub const BITMAP: &str = "image/*";
/// An EMF (or WMF) picture: what Windows apps copy as vector art (the system clipboard reads it
/// where the platform offers it).
pub const EMF: &str = "image/emf";
/// An EPS (or other PostScript) picture: what drawing and typesetting apps copy as vector art
/// beside, or instead of, a PDF (#1499).
pub const EPS: &str = "application/postscript";
/// A `.ai` or `.ait` file whose bytes are a PDF, copied in a file manager ([`file_flavour`]): Paste
/// reads it as `document.open` does, from its editing data. The type is VectorCraft's own: no
/// system clipboard carries it, and Paste asks for [`PDF`] to get it.
pub const AI: &str = "application/x-ai";
/// What Paste reads from other apps, best first: vector art before text, text before bitmaps
/// (word processors offer a picture of copied text too).
pub const PASTE_ORDER: [&str; 6] = [SVG, PDF, EPS, EMF, TEXT, BITMAP];
/// Is `text` only an address (a web page's, an image's, a file's)? A browser's Copy Image puts the
/// image's address as text next to the picture: then the picture is what Paste takes (#597). An
/// address alone pastes as text, and nothing is fetched.
pub fn is_address(text: &str) -> bool {
    const SCHEMES: [&str; 6] = ["http://", "https://", "file://", "ftp://", "data:", "blob:"];
    let t = text.trim();
    !t.chars().any(char::is_whitespace) && SCHEMES.iter().any(|s| t.get(..s.len()).is_some_and(|h| h.eq_ignore_ascii_case(s)) && t.len() > s.len())
}

/// Is `text` only a reference to a picture: an address ([`is_address`]), or markup that shows
/// nothing but images (`<img …>`)? A browser's Copy Image puts one of them next to the picture
/// (some put the image's HTML there as text, #1089): then the picture is what Paste takes.
pub fn names_a_picture(text: &str) -> bool {
    is_address(text) || only_images(text)
}

/// Is `html` markup whose only content is `<img>` elements: no text outside its tags (whitespace
/// between them aside), and at least one image?
fn only_images(html: &str) -> bool {
    let mut images = 0;
    let mut rest = html.trim();
    while !rest.is_empty() {
        let Some(tag) = rest.strip_prefix('<') else { return false };
        let Some(end) = tag.find('>') else { return false };
        let name = tag.get(..end).unwrap_or_default().split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default();
        if name.eq_ignore_ascii_case("img") {
            images += 1;
        }
        rest = tag.get(end + 1..).unwrap_or_default().trim_start();
    }
    images > 0
}

/// How much of a copied file [`file_flavour`] needs to tell what it is.
pub const FILE_HEAD: usize = 4096;

/// What a file copied in a file manager pastes as, if one of `mimes` (`image/*`: any bitmap):
/// SVG, PDF, EPS, a Windows metafile or a bitmap (its own `image/…` type), told from its first
/// [`FILE_HEAD`] bytes and its name. A `.ai` or `.ait` file is [`AI`] when its bytes are a PDF (it
/// pastes when PDF is asked for) and EPS when they are PostScript. The file's bytes are that
/// flavour's data.
pub fn file_flavour(name: &str, head: &[u8], mimes: &[&str]) -> Option<&'static str> {
    let f = fileio::detect(name, head)?;
    let (asked, mime) = match f.id {
        "svg" => (SVG, SVG),
        // `detect` names gzip data SVGZ (a compressed native file aside); `clipboard.importSvg`
        // reads SVGZ data.
        "svgz" if unpacks_to_svg(head) => (SVG, SVG),
        "pdf" => (PDF, PDF),
        "eps" => (EPS, EPS),
        "ai" | "ait" if head.starts_with(b"%PDF") => (PDF, AI),
        "ai" | "ait" if vectorcraft_pdf::is_postscript(head) => (EPS, EPS),
        "emf" | "wmf" => (EMF, EMF),
        _ if f.raster => (BITMAP, f.mime),
        _ => return None,
    };
    mimes.contains(&asked).then_some(mime)
}

/// Does gzip data `head` (a file's first bytes, often a stream cut short) unpack to SVG markup?
/// The first [`FILE_HEAD`] bytes it unpacks to are read as UTF-16 after a UTF-16 byte order mark,
/// else as UTF-8, and checked with [`looks_like_svg`].
fn unpacks_to_svg(head: &[u8]) -> bool {
    use std::io::Read as _;
    let mut raw = Vec::with_capacity(FILE_HEAD);
    // A stream cut short ends in an error after the bytes that unpacked: judge those.
    let _ = flate2::read::GzDecoder::new(head).take(FILE_HEAD as u64).read_to_end(&mut raw);
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| -> String {
        char::decode_utf16(rest.as_chunks::<2>().0.iter().map(|u| unit(*u))).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
    };
    let text = match raw.as_slice() {
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        _ => String::from_utf8_lossy(&raw).into_owned(),
    };
    looks_like_svg(&text)
}

/// One representation of the clipboard's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flavour {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// Longest side (px) of the PNG Copy offers: larger art is scaled down to fit.
const COPY_PNG_SIDE: f64 = 4096.0;
/// Most bytes of text `clipboard.importText` takes.
const MAX_TEXT: usize = 1 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "clipboard.exportPng",
            "Clipboard as PNG",
            [],
            None,
            "{scale?: 1 (pixels per point, up to 64)} → {dataBase64, width, height (px)} the copied objects cropped to their visual bounds, transparent around them (dataBase64 null when the clipboard is empty)",
            always,
            export_png
        ),
        cmd!(
            query "clipboard.exportPdf",
            "Clipboard as PDF",
            [],
            None,
            "{} → {dataBase64} a one-page PDF of the copied objects, the page their visual bounds (null when the clipboard is empty)",
            always,
            export_pdf
        ),
        cmd!(
            query "clipboard.exportText",
            "Clipboard as Text",
            [],
            None,
            "{} → {text} the copied type objects' text, one object per line (null unless every copied object is type, or a group of type)",
            always,
            export_text
        ),
        cmd!(
            query "clipboard.flavours",
            "Clipboard Formats",
            [],
            None,
            "{} → {flavours: [mime…]} what Copy offers other apps for the current clipboard, best first, by the Clipboard Handling preferences: text/plain (a type-only copy's text, else the SVG markup with copyAsSvg), image/svg+xml (copyAsSvg), application/pdf (copyAsPdf), image/png (always; PDF and PNG only for objects with an area); the app publishes them on Copy and Cut",
            always,
            flavours
        ),
        cmd!(
            query "clipboard.importImage",
            "Load Image into Clipboard",
            [],
            None,
            "{dataBase64 (PNG, JPEG, GIF, WebP, TIFF or BMP bytes), mime?: the type the system clipboard gave (an image/… type; the format is read from the bytes), center?: [x, y]} replace the clipboard with the image, embedded at 100% of its physical size (its resolution, else 72 ppi), centred on `center` (default: the first artboard) → {count, width, height (pt), pixelWidth, pixelHeight}; then run edit.paste",
            has_doc,
            import_image
        ),
        cmd!(
            query "clipboard.importText",
            "Load Text into Clipboard",
            [],
            None,
            "{text, center?: [x, y]} replace the clipboard with point text of `text` (one paragraph per line; trailing line breaks dropped) in the default type style, centred on `center` (default: the first artboard) → {count}; then run edit.paste",
            has_doc,
            import_text
        ),
        cmd!(
            query "clipboard.importPdf",
            "Load PDF into Clipboard",
            [],
            None,
            "{dataBase64, page?: 1, password? (an encrypted PDF), editingData?: false (true: the data is a .ai or .ait file, read as document.open reads it; when document.open would use its editing data, every object of that data comes in, each layer's objects in stacking order and a hidden layer's objects hidden; else page `page`), center?: [x, y]} replace the clipboard with the objects of one PDF page, or of a .ai file's editing data (with the images, patterns and swatches they use), centred on `center` (default: the first artboard) → {count, warnings}; then run edit.paste",
            has_doc,
            import_pdf
        ),
        cmd!(
            query "clipboard.importEmf",
            "Load Metafile into Clipboard",
            [],
            None,
            "{dataBase64 (EMF or WMF bytes), center?: [x, y]} replace the clipboard with the picture's objects (paths, clipping groups, images, point type; the images they use), centred on `center` (default: the first artboard) → {count, warnings}; records VectorCraft doesn't read are skipped with one warning; then run edit.paste",
            has_doc,
            import_emf
        ),
        cmd!(
            query "clipboard.importEps",
            "Load EPS into Clipboard",
            [],
            None,
            "{dataBase64 (EPS or other PostScript bytes), center?: [x, y]} replace the clipboard with the picture's objects as document.open reads an EPS (editable paths, type and images, with the images, patterns and swatches they use; a hidden layer's objects hidden), centred on `center` (default: the first artboard) → {count, warnings}; then run edit.paste",
            has_doc,
            import_eps
        ),
    ]
}

/// Is every object type (or a group of type only)?
fn type_only(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Text(_) => true,
        NodeKind::Group { children, .. } => !children.is_empty() && children.iter().all(|c| type_only(c)),
        _ => false,
    }
}

impl Clipboard {
    /// The copied type objects' text, one object per line (`None` unless it holds type only).
    pub fn text(&self) -> Option<String> {
        let nodes = &self.nodes;
        if nodes.is_empty() || !nodes.iter().all(type_only) {
            return None;
        }
        let mut lines = vec![];
        for n in nodes {
            n.walk(&mut |c| {
                if let NodeKind::Text(t) = &c.kind {
                    lines.push(t.plain_text());
                }
            });
        }
        Some(lines.join("\n"))
    }

    /// The objects as a document whose one artboard is their visual bounds (`None` when there are
    /// none or they have no extent).
    fn cropped(&self) -> Option<(Document, vectorcraft_geom::Rect)> {
        let b = self.bounds().filter(|b| b.width() > 0.0 && b.height() > 0.0)?;
        Some((fileio::with_single_artboard(self.to_document(), b, "Clipboard"), b))
    }

    /// A PNG of the objects at `scale` pixels per point → (bytes, width, height); `None` when
    /// there are none.
    pub fn png(&self, scale: f64) -> Result<Option<(Vec<u8>, u32, u32)>> {
        const C: &str = "clipboard.exportPng";
        let Some((doc, b)) = self.cropped() else { return Ok(None) };
        let (w, h) = vectorcraft_render::raster_size(b, scale).map_err(|e| bad(C, e))?;
        let png = fileio::encode(&doc, "png", &json!({ "scale": scale, "background": "transparent" }))?;
        Ok(Some((png, w, h)))
    }

    /// A one-page PDF of the objects (without the native document inside); `None` when there are
    /// none.
    pub fn pdf(&self) -> Result<Option<Vec<u8>>> {
        let Some((doc, _)) = self.cropped() else { return Ok(None) };
        fileio::encode(&doc, "pdf", &json!({ "preserveEditing": false })).map(Some)
    }

    /// The MIME types of [`Self::flavours`], without making them.
    pub fn flavour_types(&self, svg: bool, pdf: bool) -> Vec<&'static str> {
        if self.is_empty() {
            return vec![];
        }
        let text = svg || self.nodes.iter().all(type_only);
        // Art without an area (a lone guide-like line) has no page or picture.
        let area = self.bounds().is_some_and(|b| b.width() > 0.0 && b.height() > 0.0);
        [(TEXT, text), (SVG, svg), (PDF, pdf && area), (PNG, area)].into_iter().filter_map(|(m, on)| on.then_some(m)).collect()
    }

    /// What Copy offers other apps, best first, with Include SVG Code (`svg`) and On Copy: PDF
    /// (`pdf`) as given (see `clipboard.flavours`). A [`Flavour`] that can't be made (art too large
    /// to render) is left out; the PNG is scaled down to at most [`COPY_PNG_SIDE`] pixels a side.
    pub fn flavours(&self, svg: bool, pdf: bool) -> Vec<Flavour> {
        let types = self.flavour_types(svg, pdf);
        let svg = types.contains(&SVG).then(|| self.svg()).flatten();
        let mut out = vec![];
        for mime in types {
            let data = match mime {
                TEXT => self.text().or_else(|| svg.clone()).map(String::into_bytes),
                SVG => svg.clone().map(String::into_bytes),
                PDF => self.pdf().inspect_err(|e| log::warn!("clipboard PDF: {e}")).ok().flatten(),
                _ => {
                    let side = self.bounds().map_or(1.0, |b| b.width().max(b.height()));
                    let scale = (COPY_PNG_SIDE / side).clamp(0.01, 1.0);
                    self.png(scale).inspect_err(|e| log::warn!("clipboard PNG: {e}")).ok().flatten().map(|(png, ..)| png)
                }
            };
            out.extend(data.map(|data| Flavour { mime, data }));
        }
        out
    }
}

impl Session {
    /// The copied type objects' text, one object per line (`None` unless the clipboard holds type
    /// only).
    pub fn clipboard_text(&self) -> Option<String> {
        self.clipboard.text()
    }

    /// A PNG of the copied objects at `scale` pixels per point → (bytes, width, height); `None`
    /// when the clipboard is empty.
    pub fn clipboard_png(&self, scale: f64) -> Result<Option<(Vec<u8>, u32, u32)>> {
        self.clipboard.png(scale)
    }

    /// A one-page PDF of the copied objects (without the native document inside); `None` when the
    /// clipboard is empty.
    pub fn clipboard_pdf(&self) -> Result<Option<Vec<u8>>> {
        self.clipboard.pdf()
    }

    /// The MIME types of [`Self::clipboard_flavours`], without making them.
    pub fn clipboard_flavour_types(&self) -> Vec<&'static str> {
        self.clipboard.flavour_types(self.prefs.copy_as_svg, self.prefs.copy_as_pdf)
    }

    /// What Copy offers other apps, best first (see `clipboard.flavours`): [`Clipboard::flavours`]
    /// by the Clipboard Handling preferences.
    pub fn clipboard_flavours(&self) -> Vec<Flavour> {
        self.clipboard.flavours(self.prefs.copy_as_svg, self.prefs.copy_as_pdf)
    }

    /// Replace the clipboard with `clip`, centred on `p`'s `center` (default: the first
    /// artboard's centre) → the number of objects.
    pub(super) fn load_clipboard(&mut self, mut clip: Clipboard, p: &Value) -> Result<usize> {
        let center = match point_param(p, "center") {
            Some(c) => c,
            None => self.doc()?.doc.artboards.first().map(|a| a.rect.center()).unwrap_or_default(),
        };
        if let Some(b) = clip.bounds() {
            let xf = Affine::translate(center - b.center());
            for n in &mut clip.nodes {
                n.transform(xf, false);
            }
        }
        let count = clip.nodes.len();
        self.clipboard = clip;
        Ok(count)
    }
}

/// The bytes of `p`'s `dataBase64`.
fn data_param(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    let b64 = str_param(p, "dataBase64").ok_or_else(|| bad(cmd, "give dataBase64"))?;
    vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad base64"))
}

fn export_png(s: &mut Session, p: &Value) -> Result<Value> {
    let scale = f64_or(p, "scale", 1.0);
    if !(scale > 0.0 && scale <= 64.0) {
        return Err(bad("clipboard.exportPng", "scale must be a number above 0, at most 64"));
    }
    Ok(match s.clipboard_png(scale)? {
        Some((png, w, h)) => json!({ "dataBase64": vectorcraft_format::base64_encode(&png), "width": w, "height": h }),
        None => json!({ "dataBase64": null }),
    })
}

fn export_pdf(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "dataBase64": s.clipboard_pdf()?.map(|b| vectorcraft_format::base64_encode(&b)) }))
}

fn export_text(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "text": s.clipboard_text() }))
}

fn flavours(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "flavours": s.clipboard_flavour_types() }))
}

fn import_image(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "clipboard.importImage";
    if let Some(m) = str_param(p, "mime").filter(|m| !m.starts_with("image/") || *m == SVG) {
        return Err(bad(C, format!("`{m}` isn't a bitmap: use clipboard.importSvg, importPdf or importText")));
    }
    let img = fileio::raster_image(&data_param(p, C)?).map_err(|e| bad(C, e.to_string()))?;
    let (sx, sy) = pt_per_px(img.ppi);
    let (w, h) = (img.width, img.height);
    let image =
        ImageObject { key: img.key.clone(), width: w, height: h, xf: Affine::scale_non_uniform(sx, sy), link: None, placement: Default::default() };
    let mut clip = Clipboard { nodes: vec![Node::new(NodeId(1), NodeKind::Image(Box::new(image)))], ..Default::default() };
    clip.images.insert(img.key, img.blob);
    let count = s.load_clipboard(clip, p)?;
    Ok(json!({ "count": count, "width": w as f64 * sx, "height": h as f64 * sy, "pixelWidth": w, "pixelHeight": h }))
}

fn import_text(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "clipboard.importText";
    let raw = str_param(p, "text").ok_or_else(|| bad(C, "give text"))?;
    if raw.len() > MAX_TEXT {
        return Err(bad(C, format!("the text is over {} MB", MAX_TEXT >> 20)));
    }
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_end_matches('\n');
    if text.trim().is_empty() {
        return Err(bad(C, "the text is empty"));
    }
    let mut t = TextObject::point(Point::ZERO, text, super::super::create::new_type_style(s, &Value::Null));
    crate::cmd::typecmd::refresh_bounds(&mut t);
    let n = Node::new(NodeId(1), NodeKind::Text(Box::new(t)));
    let count = s.load_clipboard(Clipboard { nodes: vec![n], ..Default::default() }, p)?;
    Ok(json!({ "count": count }))
}

fn import_emf(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "clipboard.importEmf";
    let bytes = data_param(p, C)?;
    if vectorcraft_metafile::sniff(&bytes).is_none() {
        return Err(bad(C, "the data isn't an EMF or WMF picture"));
    }
    import_picture(s, p, C, "Clipboard.emf", &bytes)
}

fn import_eps(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "clipboard.importEps";
    let bytes = data_param(p, C)?;
    if !vectorcraft_pdf::is_postscript(&bytes) {
        return Err(bad(C, "the data isn't an EPS or PostScript picture"));
    }
    import_picture(s, p, C, "Clipboard.eps", &bytes)
}

/// Replace the clipboard with the objects of the picture `bytes`, read as the file `name` is
/// opened (command `cmd`).
fn import_picture(s: &mut Session, p: &Value, cmd: &str, name: &str, bytes: &[u8]) -> Result<Value> {
    let loaded = fileio::load(name, bytes).map_err(|e| bad(cmd, e.to_string()))?;
    import_loaded(s, p, cmd, loaded)
}

/// Replace the clipboard with every object of `loaded`, a file read as `document.open` reads it
/// (command `cmd`). The layers are left out, and the objects of a hidden layer come in hidden.
fn import_loaded(s: &mut Session, p: &Value, cmd: &str, mut loaded: fileio::Loaded) -> Result<Value> {
    loaded.doc.drop_edit_modes();
    for layer in loaded.doc.layers.iter_mut().filter(|l| !l.visible) {
        for object in std::sync::Arc::make_mut(layer).children_mut().into_iter().flatten() {
            std::sync::Arc::make_mut(object).visible = false;
        }
    }
    let clip = Clipboard::from_document(&loaded.doc);
    if clip.is_empty() {
        return Err(bad(cmd, "the picture has no objects"));
    }
    let count = s.load_clipboard(clip, p)?;
    Ok(json!({ "count": count, "warnings": loaded.warnings }))
}

fn import_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "clipboard.importPdf";
    let bytes = data_param(p, C)?;
    let page = match p.get("page") {
        None | Some(Value::Null) => 1,
        Some(v) => v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad(C, "page must be a whole number from 1"))? as usize,
    };
    let editing = match p.get("editingData").filter(|v| !v.is_null()) {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(C, "editingData must be true or false"))?,
    };
    let opts = fileio::LoadOptions { password: str_param(p, "password").map(str::to_string), ..Default::default() };
    // A copied .ai file pastes every object of its editing data, as document.open reads the file.
    // When document.open would not use that copy, the file pastes its page, as a PDF does.
    if editing
        && let Ok(loaded) = fileio::load_with("Clipboard.ai", &bytes, &opts)
        && (loaded.editing_data || loaded.restored)
    {
        return import_loaded(s, p, C, loaded);
    }
    let (mut doc, warnings) = fileio::page_document(&bytes, page - 1, &opts).map_err(|e| bad(C, e.to_string()))?;
    doc.drop_edit_modes();
    let clip = Clipboard::from_document(&doc);
    if clip.is_empty() {
        return Err(bad(C, format!("page {page} of the PDF has no objects")));
    }
    let count = s.load_clipboard(clip, p)?;
    Ok(json!({ "count": count, "warnings": warnings }))
}
