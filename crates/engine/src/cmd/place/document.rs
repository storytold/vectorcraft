//! Placed documents (`file.place` of a VectorCraft document, a PDF, a `.ai` file or an EPS file
//! from a file, with `link`): one locked object showing an artboard of the document, a page of the
//! PDF or the EPS file's art, a [`PlacedDocument`].
//!
//! The object keeps the file's bytes (in the document's images, like image data) and the link; the
//! art it shows is read from them the way placing the file without Link reads it
//! ([`document_art`]: the artboard's or page's art, with the file's own linked images found), so
//! Break Link and Object › Expand ([`expand`]) give exactly that editable copy. The Links commands
//! ([`super::super::links`]) tell when the file changed and read it again.
//!
//! Each placed file also gets a preview ([`PREVIEW_PX_PER_PT`]; JPEG when opaque, else PNG), kept
//! as the blob's [`ImageBlob::proxy`]: a save writes only the preview (unless Include Linked
//! Files), as for linked images. The file is read again when it is needed ([`read_file_again`]:
//! output, or the screen zoomed past the preview), while it is unchanged, and output falls back to
//! the preview with a warning when it can't be ([`full_documents`]).

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use vectorcraft_doc::links::hash_bytes;
use vectorcraft_doc::placed_document::{PREVIEW_MAX, PREVIEW_PX_PER_PT, key_for};
use vectorcraft_doc::{ColorMode, Document, ImageBlob, LinkInfo, Node, NodeKind, PageBox, PlacedDocument, PlacementOptions};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_pdf::CropTo;

use super::super::fileio;
use super::super::{EngineError, NodeId, Result, bad};

/// The largest file placed (bytes).
pub(crate) const MAX_FILE: usize = 256 << 20;
/// The format ids of VectorCraft documents.
const NATIVE: [&str; 2] = ["vectorcraft", "template"];
/// The type a placed VectorCraft document's bytes are stored with (a PDF's: its format's).
const MIME: &str = "application/json";
/// The type a placed EPS or PostScript file's bytes are stored with.
pub(crate) const POSTSCRIPT: &str = "application/postscript";
/// What follows the hash in the key of a placed PostScript `.ai` file ([`key_for`]).
const AI_MARK: &str = "-ai";

/// Is `key` (of a placed document) a PostScript `.ai` file's ([`AI_MARK`])? Hashes are hex, so
/// only the mark puts `-ai-` in a key.
pub(crate) fn is_postscript_ai_key(key: &str) -> bool {
    key.contains(&format!("{AI_MARK}-"))
}

/// Is `name` with `bytes` a VectorCraft document?
pub(crate) fn is_native(name: &str, bytes: &[u8]) -> bool {
    fileio::detect(name, bytes).is_some_and(|f| NATIVE.contains(&f.id))
}

/// Is `name` with `bytes` a PDF, or a `.ai` or `.ait` file read as PDF pages (not PostScript)?
pub(crate) fn is_pdf(name: &str, bytes: &[u8]) -> bool {
    fileio::detect(name, bytes).is_some_and(|f| matches!(f.id, "pdf" | "ai" | "ait")) && !vectorcraft_pdf::is_postscript(bytes)
}

/// Is `bytes` an EPS file, a PostScript file or a PostScript `.ai` or `.ait` file, read by the EPS
/// importer (not a DCS file, which is placed as a linked image)?
pub(crate) fn is_postscript(bytes: &[u8]) -> bool {
    vectorcraft_pdf::is_postscript(bytes) && !super::dcs::is_dcs(bytes)
}

/// The editing data of the `.ai` file `name` (`bytes`) when its PDF part is only a placeholder, or
/// can't be read:
/// the document its pages are read from ([`super::pdf_art`]), with the files it links. `None` for
/// other files.
pub(crate) fn editing_data(name: &str, bytes: &[u8]) -> Option<Document> {
    if !is_pdf(name, bytes) {
        return None;
    }
    let opts = fileio::LoadOptions::default();
    let read = fileio::page_document(bytes, 0, &opts);
    match fileio::page_or_editing_data(name, bytes, &opts, 0, read) {
        Ok((doc, _, true)) => Some(doc),
        _ => None,
    }
}

/// The type the bytes of placed file `name` (`bytes`) are stored with: a PDF's format's,
/// [`POSTSCRIPT`] for an EPS or PostScript file, else [`MIME`].
fn mime(name: &str, bytes: &[u8]) -> &'static str {
    if is_postscript(bytes) {
        return POSTSCRIPT;
    }
    fileio::detect(name, bytes).filter(|_| is_pdf(name, bytes)).map_or(MIME, |f| f.mime)
}

/// The box of a PDF page placed linked for File › Place's `crop` (Bounding Box: the crop box,
/// the art's bounds being [`PlacedDocument::bounding`]).
pub(crate) fn page_box(crop: CropTo) -> PageBox {
    match crop {
        CropTo::Art => PageBox::Art,
        CropTo::Trim => PageBox::Trim,
        CropTo::Bleed => PageBox::Bleed,
        CropTo::Media => PageBox::Media,
        CropTo::Crop | CropTo::Bounding => PageBox::Crop,
    }
}

/// The `crop` a PDF page placed linked showing `b` is read with.
fn crop_to(b: PageBox) -> CropTo {
    match b {
        PageBox::Crop => CropTo::Crop,
        PageBox::Art => CropTo::Art,
        PageBox::Trim => CropTo::Trim,
        PageBox::Bleed => CropTo::Bleed,
        PageBox::Media => CropTo::Media,
    }
}

/// An artboard of a VectorCraft document, or a page of a PDF or `.ai` file, read as art to place.
pub(crate) struct DocumentArt {
    /// The document, its layers taken: the art's resources.
    pub doc: Document,
    /// The art on the artboard (guides, hidden and template layers left out).
    pub nodes: Vec<Node>,
    /// The artboard the art is clipped to (`None`: the art's bounds are shown).
    pub clip: Option<Rect>,
    /// What is shown: the artboard, or the art's bounds.
    pub frame: Rect,
    pub warnings: Vec<String>,
    /// Whether a PDF or `.ai` page was read from the file's editing data (its page only a
    /// placeholder, or its PDF part unreadable) instead of from the page.
    pub editing_data: bool,
}

/// Artboard `page` (1-based) of the VectorCraft document `name` (`bytes`, read from `path` when
/// there is one, so its linked images are found), clipped to the artboard unless `bounding`; or
/// page `page` of the PDF or `.ai` file `name`, clipped to its box `page_box` unless `bounding`
/// ([`super::pdf_art`]); or the art of the EPS or PostScript file `name` ([`is_postscript`]) on
/// its page `page`, clipped to its bounding box unless `bounding`, as placing it without Link reads
/// it.
pub(crate) fn document_art(
    name: &str,
    bytes: &[u8],
    path: Option<&str>,
    page: u32,
    bounding: bool,
    page_box: PageBox,
    cmd: &str,
) -> Result<DocumentArt> {
    opened_document_art(name, (bytes, None), path, page, bounding, page_box, cmd)
}

/// [`document_art`] of `bytes` and, for a PDF already read, `opened`: the PDF `bytes` as
/// [`vectorcraft_pdf::open_pdf`] read it without a password, so its page is read without reading
/// the file again.
fn opened_document_art(
    name: &str,
    (bytes, opened): (&[u8], Option<&vectorcraft_pdf::Opened>),
    path: Option<&str>,
    page: u32,
    bounding: bool,
    page_box: PageBox,
    cmd: &str,
) -> Result<DocumentArt> {
    let file = fileio::file_name(name);
    let pdf = is_pdf(name, bytes);
    if !pdf && !is_native(name, bytes) && !is_postscript(bytes) {
        return Err(bad(cmd, format!("`{file}` isn't a VectorCraft document, a PDF or a PDF-compatible .ai file")));
    }
    if bytes.len() > MAX_FILE {
        return Err(bad(cmd, format!("`{file}` is over {} MB", MAX_FILE >> 20)));
    }
    if pdf {
        let o = fileio::LoadOptions { crop: crop_to(page_box), ..Default::default() };
        return super::pdf_art(name, (bytes, opened), path, page as usize, o, bounding, cmd);
    }
    // A VectorCraft document, or an EPS or PostScript file: the document our EPS files carry, else
    // what its PostScript draws.
    let mut l = fileio::load(name, bytes)?;
    // Its linked images show their files; its placed documents stay as they are, unless their
    // stored bytes are lost, when resolving reads their files, which can place this one again.
    // A document already being read further up, or past [`MAX_NESTED_READS`] of them, has its links
    // left as stored.
    if let Some(_reading) = Reading::enter(bytes) {
        super::super::links::resolve(&mut l.doc, super::super::links::LinkBase::file(path), false, false);
    }
    l.doc.drop_edit_modes();
    // The native file does not store type's layout bounds. Measure them before cropping or
    // copying the art, so the placed group keeps the positions of aligned point type.
    for warning in crate::Session::refresh_text_bounds(&mut l.doc) {
        if !l.warnings.contains(&warning) {
            l.warnings.push(warning);
        }
    }
    let n = l.doc.artboards.len();
    let board = (page as usize).checked_sub(1).and_then(|i| l.doc.artboards.get(i)).map(|a| a.rect);
    let artboards = if n == 1 { "artboard" } else { "artboards" };
    let board = board.ok_or_else(|| bad(cmd, format!("page {page}: `{file}` has {n} {artboards}")))?;
    let layers = std::mem::take(&mut l.doc.layers);
    let symbols = l.doc.symbol_boxes();
    let nodes = super::on_artboard(&symbols, &layers, board);
    let frame = if bounding { super::shown_bounds(&symbols, &nodes) } else { Some(board) };
    let frame = frame.filter(|r| r.width() > 0.0 || r.height() > 0.0).ok_or_else(|| bad(cmd, format!("`{file}` has no art to place")))?;
    Ok(DocumentArt { doc: l.doc, nodes, clip: (!bounding).then_some(board), frame, warnings: l.warnings, editing_data: false })
}

/// The most placed documents read inside one another while their links are resolved.
const MAX_NESTED_READS: usize = 8;

std::thread_local! {
    /// The hashes of the documents whose links are being resolved on this thread, outermost first.
    static READING: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A document's links being resolved ([`READING`]): the entry goes when this does.
pub(super) struct Reading;

impl Reading {
    /// `None` when the document of `bytes` is already being read, or [`MAX_NESTED_READS`] are.
    pub(super) fn enter(bytes: &[u8]) -> Option<Self> {
        let hash = hash_bytes(bytes);
        READING.with_borrow_mut(|r| (r.len() < MAX_NESTED_READS && !r.contains(&hash)).then(|| r.push(hash))).map(|()| Reading)
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        READING.with_borrow_mut(|r| {
            r.pop();
        });
    }
}

/// `nodes` as one group, clipped to `clip` (its own ids: what drawing reads).
fn group(nodes: Vec<Node>, clip: Option<Rect>) -> Node {
    let mut children: Vec<Arc<Node>> = nodes.into_iter().map(Arc::new).collect();
    if let Some(r) = clip {
        children.insert(0, Arc::new(super::clip_path(NodeId(0), r)));
    }
    Node::new(NodeId(0), NodeKind::Group { children, clip: clip.is_some() })
}

/// `p`'s file (`bytes`) read as the art it shows: how the document model reads placed documents
/// ([`vectorcraft_doc::placed_document::set_loader`]).
pub(crate) fn read_document(p: &PlacedDocument, bytes: &[u8]) -> Option<(Document, Node, Rect)> {
    let a = document_art(p.link.name(), bytes, Some(&p.link.path), p.page(), p.bounding, p.page_box, "draw").ok()?;
    Some((a.doc, group(a.nodes, a.clip), a.frame))
}

/// `p`'s file read again, while it is the file it was read from (how the document model gets it
/// when only the preview is stored, [`vectorcraft_doc::placed_document::set_file_reader`]).
pub(crate) fn read_file_again(p: &PlacedDocument) -> Option<Vec<u8>> {
    // The stored path is followed as stored paths are, but with no document to share a network
    // folder with: a file on another computer is never read here, even one resolving refused
    // (which leaves the stored path in place), so embedding or exporting can't connect to it.
    let path = fileio::document_path(&p.link.path, None).ok()?;
    // One byte over the limit tells a file that is too big, without reading it all.
    let bytes = fileio::read_file_head(&path, MAX_FILE + 1).ok()?;
    if bytes.len() > MAX_FILE {
        return None;
    }
    if p.link.hash.as_ref().is_some_and(|h| *h != hash_bytes(&bytes)) {
        return None;
    }
    Some(bytes)
}

/// A VectorCraft document, or a PDF or `.ai` file, read for placing linked.
#[derive(Clone)]
pub(crate) struct Source {
    /// The file's bytes, with their preview.
    pub blob: ImageBlob,
    /// The blob's key ([`key_for`]).
    pub key: String,
    /// The box of a PDF page shown (unless the art's bounds are).
    pub page_box: PageBox,
    /// What is shown, in the file's coordinates: the artboard, the page's box or the art's bounds.
    pub frame: Rect,
    pub warnings: Vec<String>,
    /// The colour mode the file's art was read in (a PDF page painted mostly in CMYK reads in CMYK).
    pub color_mode: ColorMode,
    /// [`DocumentArt::editing_data`].
    pub editing_data: bool,
}

/// The VectorCraft document `name` (`bytes`, from `path` when there is one) read for showing
/// artboard `page`, or the PDF or `.ai` file for showing page `page` clipped to its box
/// `page_box` (its art's bounds when `bounding`).
pub(crate) fn source(name: &str, bytes: &[u8], path: Option<&str>, page: u32, bounding: bool, page_box: PageBox, cmd: &str) -> Result<Source> {
    read_source(name, bytes, None, path, (page, bounding, page_box), cmd)
}

/// A PDF (or PDF-compatible `.ai`) whose pages are read for placing linked, read once for them all.
#[derive(Clone, Copy)]
pub(crate) struct SharedFile<'a> {
    /// The file's bytes, which every page's blob keeps without a copy of its own.
    pub bytes: &'a Arc<Vec<u8>>,
    /// The bytes' hash ([`hash_bytes`]).
    pub hash: &'a str,
    /// The bytes as [`vectorcraft_pdf::open_pdf`] read them, without a password.
    pub opened: &'a vectorcraft_pdf::Opened,
}

/// [`source`] of page `page` of `file`: several pages of one file read this way share one copy of
/// it, and the file is hashed and read once for them all.
pub(crate) fn shared_source(
    name: &str,
    file: SharedFile<'_>,
    path: Option<&str>,
    page: u32,
    bounding: bool,
    page_box: PageBox,
    cmd: &str,
) -> Result<Source> {
    read_source(name, file.bytes, Some(file), path, (page, bounding, page_box), cmd)
}

/// [`source`], the blob keeping `shared`'s bytes when given (else a copy of `bytes`), its hash
/// used for the key (else `bytes` hashed) and its pages read from what was read of it; `shown` is
/// the page (or artboard), whether the art's bounds are shown, and the page's box.
fn read_source(
    name: &str,
    bytes: &[u8],
    shared: Option<SharedFile<'_>>,
    path: Option<&str>,
    shown: (u32, bool, PageBox),
    cmd: &str,
) -> Result<Source> {
    let (page, bounding, page_box) = shown;
    let hash = shared.map_or_else(|| hash_bytes(bytes), |f| f.hash.to_string());
    let opened = shared.map(|f| f.opened);
    let shared = shared.map(|f| f.bytes);
    // A PostScript `.ai` is read with the artboard its header gives, an EPS file on its page: the
    // same bytes under the two names show different art, so they have keys of their own.
    let ai = is_postscript(bytes) && fileio::detect(name, bytes).is_some_and(|f| matches!(f.id, "ai" | "ait"));
    let hash = if ai { format!("{hash}{AI_MARK}") } else { hash };
    // The same document placed the same way is read once (placing it again is free).
    let key = key_for(&hash, page, bounding, page_box);
    // Resolving its links depends on where the file is and on what the caller may read, so a hit
    // needs the same path, and nothing is shared with or from a confined (automation) caller.
    let confined = crate::file_access::current().is_some();
    if !confined && let Some((_, mut s)) = lock_memo().iter().find(|(at, s)| s.key == key && at.as_deref() == path).cloned() {
        // The same bytes: the copy given is kept instead, so the pages that share it go on sharing.
        if let Some(b) = shared {
            s.blob.bytes = b.clone();
        }
        return Ok(s);
    }
    let a = opened_document_art(name, (bytes, opened), path, page, bounding, page_box, cmd)?;
    let bytes = shared.cloned().unwrap_or_else(|| Arc::new(bytes.to_vec()));
    let mut blob = ImageBlob { mime: mime(name, &bytes).into(), bytes, proxy: None, file_font: Default::default() };
    let shown = PlacedDocument {
        link: LinkInfo { page: Some(page), ..LinkInfo::new(path.unwrap_or(name)) },
        key: key.clone(),
        bounding,
        page_box,
        width: a.frame.width(),
        height: a.frame.height(),
        xf: Affine::IDENTITY,
        placement: PlacementOptions::default(),
    };
    let art = group(a.nodes, a.clip);
    blob.proxy = preview(&a.doc, &art, a.frame).map(Arc::new);
    let (color_mode, editing_data) = (a.doc.color_mode, a.editing_data);
    // Drawing it needn't read it again.
    vectorcraft_doc::placed_document::prime(&shown, blob.bytes.clone(), (a.doc, art, a.frame));
    let s = Source { blob, key, page_box, frame: a.frame, warnings: a.warnings, color_mode, editing_data };
    if !confined {
        let mut m = lock_memo();
        if m.len() >= MEMO_SIZE {
            m.remove(0);
        }
        m.push((path.map(str::to_string), s.clone()));
    }
    Ok(s)
}

/// The most files remembered.
const MEMO_SIZE: usize = 32;

/// Files read lately for placing, by key.
static MEMO: std::sync::Mutex<Vec<(Option<String>, Source)>> = std::sync::Mutex::new(Vec::new());

fn lock_memo() -> std::sync::MutexGuard<'static, Vec<(Option<String>, Source)>> {
    // Entries are only pushed and removed: a panic while held leaves it whole.
    MEMO.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The preview of `art` (of document `doc`, showing `frame`): [`PREVIEW_PX_PER_PT`], at most
/// [`PREVIEW_MAX`] pixels a side; JPEG when opaque (inside its outer pixels), else PNG.
fn preview(doc: &Document, art: &Node, frame: Rect) -> Option<Vec<u8>> {
    let k = PREVIEW_PX_PER_PT.min(PREVIEW_MAX as f64 / frame.width().max(frame.height()).max(1e-9));
    let side = |v: f64| (v * k).round().clamp(1.0, PREVIEW_MAX as f64) as u32;
    let (w, h) = (side(frame.width()), side(frame.height()));
    let view = Affine::scale_non_uniform(w as f64 / frame.width().max(1e-9), h as f64 / frame.height().max(1e-9))
        * Affine::translate((-frame.x0, -frame.y0));
    let mut r = vectorcraft_render::Renderer::new();
    let img = vectorcraft_render::placed_document::render_inside(&mut r, doc, &Arc::new(art.clone()), w as u16, h as u16, view)?;
    // Premultiplied → straight.
    let px = img
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[r, g, b, a]| {
            let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
            [un(r), un(g), un(b), a]
        })
        .collect();
    let rgba = image::RgbaImage::from_raw(w, h, px)?;
    let inner = |x: u32, y: u32| x > 0 && y > 0 && x + 1 < w && y + 1 < h;
    if rgba.enumerate_pixels().all(|(x, y, p)| p[3] >= 250 || !inner(x, y)) {
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let raw = craft_image_codecs::Image::from_raw(
            rgb.width(),
            rgb.height(),
            craft_image_codecs::ChannelLayout::Rgb,
            craft_image_codecs::SampleType::U8,
            rgb.into_raw(),
        )
        .ok()?;
        let opts = craft_image_codecs::EncodeOptions { jpeg_quality: 80, ..Default::default() };
        craft_image_codecs::encode(&raw, craft_image_codecs::Format::Jpeg, &opts).ok()
    } else {
        let raw = craft_image_codecs::Image::from_raw(
            rgba.width(),
            rgba.height(),
            craft_image_codecs::ChannelLayout::Rgba,
            craft_image_codecs::SampleType::U8,
            rgba.into_raw(),
        )
        .ok()?;
        craft_image_codecs::encode(&raw, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).ok()
    }
}

/// `s` stored in `d` (objects showing the same thing share one blob).
pub(crate) fn store(d: &mut Document, s: &Source) {
    d.images.entry(s.key.clone()).or_insert_with(|| s.blob.clone());
}

/// A new placed document of `d` showing `s` at 100%, where its art lies in the file.
pub(crate) fn node(d: &mut Document, s: &Source, link: LinkInfo, bounding: bool) -> Node {
    store(d, s);
    let placed = PlacedDocument {
        link,
        key: s.key.clone(),
        bounding,
        page_box: s.page_box,
        width: s.frame.width(),
        height: s.frame.height(),
        xf: Affine::translate((s.frame.x0, s.frame.y0)),
        placement: PlacementOptions::default(),
    };
    Node::new(d.alloc_id(), NodeKind::PlacedDocument(Box::new(placed)))
}

/// `d` for output: linked images and placed documents that keep only their previews get their
/// files back, else output as their previews, with a warning. The art of the stored brushes counts
/// too.
pub(crate) fn full_documents(d: &Document) -> (Cow<'_, Document>, Vec<String>) {
    let (d, mut warnings) = super::super::links::full_linked_images(d, Default::default());
    let doc = match &d {
        Cow::Borrowed(doc) => doc,
        Cow::Owned(doc) => doc,
    };
    let mut need: Vec<PlacedDocument> = vec![];
    let mut seen = std::collections::HashSet::new();
    super::super::links::with_brush_art(doc).visit_placed(|_, p| {
        if doc.images.get(&p.key).is_some_and(ImageBlob::is_proxy) && seen.insert(p.key.clone()) {
            need.push(p.clone());
        }
    });
    if need.is_empty() {
        return (d, warnings);
    }
    let mut out = match d {
        Cow::Borrowed(doc) => doc.clone(),
        Cow::Owned(doc) => doc,
    };
    let mut previews = BTreeSet::new();
    for p in need {
        let Some(blob) = out.images.get(&p.key) else { continue };
        match vectorcraft_doc::placed_document::full_bytes(&p, blob) {
            Some(bytes) => {
                out.images.insert(
                    p.key.clone(),
                    ImageBlob { mime: mime(p.link.name(), &bytes).into(), bytes, proxy: blob.proxy.clone(), file_font: Default::default() },
                );
            }
            None => {
                previews.insert(p.key);
            }
        }
    }
    if !previews.is_empty() {
        warnings.push(match previews.len() {
            1 => "1 placed document output as its preview: its file can't be found, or changed since it was read (relink or update it)".to_string(),
            n => format!(
                "{n} placed documents output as their previews: their files can't be found, or changed since they were read (relink or update them)"
            ),
        });
        super::super::links::editing_brush_art(&mut out, |out| out.placed_as_previews(&previews));
    }
    (Cow::Owned(out), warnings)
}

/// Placed document `id` of `d` replaced by an editable copy of its art, as placing its file
/// without Link gives (its resources joining `d`, renamed where `d` uses a name for something
/// else), keeping the object's id, place, name and transparency: Break Link and Object › Expand.
/// A placed document in opacity-mask art or in the art of a symbol, graph design or pattern is
/// replaced where it is stored.
pub(crate) fn expand(d: &mut Document, id: NodeId, cmd: &str) -> Result<()> {
    let n = d.stored_node(id).map(|(n, _)| Node::clone(n)).ok_or(EngineError::NoNode(id))?;
    let NodeKind::PlacedDocument(p) = &n.kind else { return Err(bad(cmd, format!("object {} is not a placed document", id.0))) };
    let blob = d.images.get(&p.key).ok_or_else(|| bad(cmd, format!("object {} has lost its file's contents: relink it", id.0)))?;
    let bytes = vectorcraft_doc::placed_document::full_bytes(p, blob)
        .ok_or_else(|| bad(cmd, format!("{} can't be read, or changed since it was read: relink or update it first", p.link.path)))?;
    let a = document_art(p.link.name(), &bytes, Some(&p.link.path), p.page(), p.bounding, p.page_box, cmd)?;
    let m = p.art_xf(a.frame).ok_or_else(|| bad(cmd, format!("object {} has no size", id.0)))?;
    let mut nodes = a.nodes;
    super::adopt::adopt(d, &a.doc, &mut nodes);
    let mut children: Vec<Arc<Node>> = nodes.iter().map(|n| Arc::new(d.reid(n))).collect();
    if let Some(r) = a.clip {
        children.insert(0, Arc::new(super::clip_path(d.alloc_id(), r)));
    }
    let art = super::transformed(Node::new(d.alloc_id(), NodeKind::Group { children, clip: a.clip.is_some() }), m);
    let mut out = n.clone();
    out.kind = art.kind;
    if d.node(id).is_none() {
        *d.stored_node_mut(id).ok_or(EngineError::NoNode(id))? = out;
        return Ok(());
    }
    let (par, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    d.remove(id)?;
    d.insert(par, index, out)?;
    Ok(())
}
