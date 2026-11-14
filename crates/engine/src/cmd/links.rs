//! Linked images (File → Place with Link): finding their files, telling when they changed, and
//! reading them again.
//!
//! A linked image keeps its file's absolute path, the path relative to the document (written on
//! save), the file's size, modification time and hash ([`LinkInfo`]), and a low-resolution
//! preview that the document saves in place of the pixels ([`ImageBlob::proxy`]). A file is looked
//! for at its path, then at its relative path and by name in the document's folder. When a
//! document opens ([`resolve`]), the files found unchanged are read again, modified ones follow
//! Preferences → File Handling → Update Links, and missing ones show their preview.
//!
//! `links.check` reports, `links.update` reads modified files again, `links.relink` points images
//! at other files. A file read again takes the image's place as its Placement Options say
//! ([`PlacementOptions`]: keep the bounds or the transforms, fit or fill the bounds…).
//!
//! The Links panel lists every image (`links.list`) with its Link Info (`links.info`); Go To
//! selects one (`links.goTo`), Embed keeps a linked file's pixels in the document
//! (`links.embed`) and Unembed writes an embedded image to a file it then links to
//! (`links.unembed`).
//!
//! Placed documents ([`PlacedDocument`], VectorCraft documents and PDF or `.ai` pages placed with
//! Link) link to their files the same way: checking, updating, relinking and finding them on open
//! work alike, and they are listed. Embed (Break Link) turns one into an editable copy of its art.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::links::{Align, Preserve, hash_bytes};
use vectorcraft_doc::{
    Appearance, DataValue, Document, ImageBlob, ImageObject, LinkInfo, Node, NodeId, NodeKind, PageBox, PlacedDocument, PlacementOptions,
    StoredPlace, StoredRoot,
};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, shapes};

use super::fileio::{self, RasterImage, absolute_path, file_created, file_stamp, read_file, write_file};
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, cmd, has_doc, ids_param, object_param, str_param, switch_or};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "links.check",
            "Check Links",
            [],
            None,
            "{ids?: [image or placed document ids] (default: every linked image and placed document, in the layers and in opacity-mask art, symbols, graph designs and patterns)} where each linked file is and whether it changed since it was read → {links: [{name, path?, status: ok|modified|missing, ids, found?: path (found away from its path: relative to the document, or by name in its folder), preview: true (showing the low-resolution preview, not the file), refused?: why the path is not followed (another computer, a device or a URL: it counts as missing)}], missing, modified}; under automation read roots, path and found are left out when the file they name is outside the roots",
            has_doc,
            check
        ),
        cmd!(
            "links.update",
            "Update Links",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the linked images whose file was modified or is showing its preview, and placed documents whose file was modified)} read their linked files again; each keeps its bounds. Images and placed documents inside symbols, graph designs, patterns and opacity masks have ids of their own, as links.list gives them; those that show the same content as one read again are read again too. One undo step → {updated: [ids], missing: [ids], warnings?: [a DCS file's spot plate whose ink a swatch of the document already names, as file.place warns]}",
            has_doc,
            update
        ),
        cmd!(
            "links.updateChanged",
            "Update Changed Links",
            [],
            None,
            "{} read again the active document's linked files that changed on disk since they were last seen (size or modification time), whatever Preferences → File Handling → Update Links says; the desktop app looks every two seconds and follows that preference. Files seen for the first time are only remembered. One undo step → {updated: [ids], ask: [], modified: []}",
            has_doc,
            crate::link_watch::update_changed
        ),
        cmd!(
            "links.relink",
            "Relink",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected ones; with folder and nothing selected, every missing link), path | folder, scope?: selected (default: ids) | all (ids and every other object linked to the same file as one of them, in the layers, opacity-mask art, symbols, graph designs and patterns) | missingInFolder (with path: ids, then every other missing link whose file name is in path's folder, each to the file of its name there; names not there are in notFound; with copy, only images are looked for) | each (with paths), paths?: [{ids, path}] (with scope each, in place of ids, path and folder: each link to its own file; an id in two entries is an error), copy?: false (with path or paths, for images of a saved document: copy the file into the document's folder and link to the copy; the names Name.png, Name 2.png … Name 99.png are tried in turn until one is free, which gets the copy, or is a file with the same contents, which is used; an error for a DCS file that reads plate files of its own), preserve?: transforms|bounds|fileDimensions|fit|fill (how the new file takes each image's place this time, as links.placementOptions describes; default: each image's Placement Options; fit keeps the new file's aspect ratio inside the old bounds: Replace)} link images (or placed documents, each showing the same artboard or page of the new file) to the file at path, or each to the file of its link's name in folder; embedded images become linked; each keeps its bounds. One undo step → {relinked: [ids], notFound: [file names], copied?: [{from, to}], kept?: [{from, to}] (with copy; to is the file name alone when the document's folder is outside the automation read roots), warnings?: (as links.update's)}",
            has_doc,
            relink
        ),
        cmd!(
            query "links.list",
            "Links",
            [],
            None,
            "{show?: all (default)|missing|modified|embedded, sort?: name|kind (file format)|status (missing, modified, ok, embedded); default: stacking order; top first, dcsTransparency?: false (Show DCS Transparency Interactions: mark the images placed from DCS files that are transparent, themselves or a group or layer holding them, or that overlap a visible object with its own transparency; it looks at every object, so it takes longer)} every image object and placed document the document stores, as the Links panel lists them: those in the layers, then those in opacity-mask art and in the art of symbols, graph designs and patterns (one row per stored object, never one per symbol instance or pattern tile) → {links: [{id, name, linked, status: ok|modified|missing|embedded, format (DCS for an image placed from a DCS file), pixelWidth, pixelHeight, path?, found?: path (found away from its path), page?, preview?: true (showing the saved preview), note?: said of an image past the decode allowance (shown at reduced size, or too large to draw), document?: true (a placed document: pageWidth and pageHeight in pt instead of pixels), dcsTransparency?: true (with dcsTransparency), owner?: {kind: layers|symbol|graphDesign|pattern, name? (of the symbol, graph design or pattern), maskOf?: id (in the opacity-mask art of that object)} (an object outside the layer tree)}], missing, modified, embedded}; under automation read roots, path and found are left out when the file they name is outside the roots",
            has_doc,
            list
        ),
        cmd!(
            "links.goTo",
            "Go To Link",
            [],
            None,
            "{id?} select image or placed document `id` (default: the first selected one); the Links panel scrolls it into view → {id, bounds: [x0, y0, x1, y1], onCanvas: true}; one in opacity-mask art or in the art of a symbol, graph design or pattern has no place of its own on the canvas: the selection is cleared → {id, onCanvas: false, owner} (owner as links.list gives it)",
            has_doc,
            go_to
        ),
        cmd!(
            "links.embed",
            "Embed Image",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected linked ones)} break the links (the Links panel's Break Link): images keep the linked files' pixels in the document; a placed document becomes an editable copy of its art, as placing its file without link gives (its symbols, patterns, swatches and images joining the document). An image whose file can't be found and that shows the saved preview, or a placed document whose file can't be read or changed, stays linked: relink or update it first. An image placed from a DCS file can't be embedded (an error, and nothing changes). One undo step → {embedded: [ids], missing: [ids]}",
            has_doc,
            embed
        ),
        cmd!(
            "links.unembed",
            "Unembed",
            [],
            None,
            "{id?: an embedded image (default: the one selected), path?} write the image's pixels to the file at path (PNG, JPEG, GIF or WebP as stored, other images as PNG; a path ending in .png/.jpg/.jpeg/.gif/.webp/.tif/.tiff/.bmp converts them) and link the image to it, keeping its bounds. One undo step → {id, path}; no path → {name, dataBase64} (nothing changes: there is no file to link to)",
            has_doc,
            unembed
        ),
        cmd!(
            query "links.info",
            "Link Info",
            [],
            None,
            "{id?} an image's or placed document's Link Info (default: the one selected) → image.info's fields (id, name, linked, link, colorMode, pixelWidth, pixelHeight, width, height) and status: ok|modified|missing|embedded, format, ppi: [x, y] (the file's), effectivePpi: [x, y] (at its placed size), scale: [x%, y%] (of its 100% size), rotation (degrees, counter-clockwise), placement: {preserve, align, clip}; linked images also fileName, location (its folder), page?, fileSize? (bytes), modified?, created? (ms since the Unix epoch), refused? (why the path is not followed); a placed document has document: true, pageWidth and pageHeight (pt) instead of pixels, ppi and colorMode, and its scale is of its artboard's or page's 100% size; under automation read roots, a link whose file is outside the roots has no link, path or location, and fileSize and modified come only from the file",
            has_doc,
            info
        ),
        cmd!(
            "links.placementOptions",
            "Placement Options",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected ones), preserve?: transforms|bounds (default)|fileDimensions|fit|fill, align?: topLeft|top|topRight|left|center (default)|right|bottomLeft|bottom|bottomRight (where the new art sits; not for bounds), clip?: false (clip it to the old bounds where it is larger)} how a file read again (links.relink, links.update) takes each image's place: transforms keeps its scale (relative to each file's 100% size), rotation and position; bounds stretches it into the old bounds; fileDimensions puts it at 100%, unrotated; fit and fill scale it proportionally to fit inside or cover the old bounds. Without options → {placement} of the first image; else one undo step → {ids, placement}",
            has_doc,
            placement_options
        ),
    ]
}

// ---------- finding files ----------

/// What became of a linked file since it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Modified,
    Missing,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Modified => "modified",
            Status::Missing => "missing",
        }
    }
}

/// The image objects that show one linked file as one content (same path, same image), or the
/// placed documents that show one artboard or page of it.
struct Group {
    link: LinkInfo,
    key: String,
    /// The stored objects showing it: in the layers, in opacity-mask art and in the art of symbols,
    /// graph designs and patterns ([`Document::visit_stored`]), each once.
    ids: Vec<NodeId>,
    /// Placed documents: what they show.
    document: Option<Shown>,
    /// An image placed from a DCS file: the plate files of their own its plates were read from.
    plate_files: Vec<vectorcraft_doc::separated::PlateFile>,
}

/// What a placed document shows: whether the art's bounds (not the artboard or page) are shown,
/// and a PDF page's box.
type Shown = (bool, PageBox);

/// The linked object `n` as (link, content key, what a placed document shows).
fn linked(n: &Node) -> Option<(&LinkInfo, &str, Option<Shown>)> {
    match &n.kind {
        NodeKind::Image(im) => Some((im.link.as_ref()?, &im.key, None)),
        NodeKind::PlacedDocument(p) => Some((&p.link, &p.key, Some((p.bounding, p.page_box)))),
        _ => None,
    }
}

/// Every linked file of `d` with the ids of the objects showing it, wherever they are stored (for
/// [`crate::link_watch`]): an image placed from a DCS file has its plate files of their own too,
/// beside its file.
pub(crate) fn linked_groups(d: &Document) -> Vec<(String, Vec<u64>)> {
    let mut out = vec![];
    for g in groups(d, None).into_iter().filter(|g| !g.ids.is_empty()) {
        let ids = ids_json(&g.ids);
        let plates: Vec<String> = g.plate_files.iter().filter_map(|f| super::place::dcs::plate_file_path(&g.link.path, &f.name)).collect();
        out.push((g.link.path, ids.clone()));
        out.extend(plates.into_iter().map(|p| (p, ids.clone())));
    }
    out
}

/// The linked images and placed documents of `d` by file and content, wherever they are stored
/// (only `ids` when given), in [`Document::visit_stored`] order. An object whose id an object
/// visited before it has (art being edited on a layer and its definition) counts once, by its
/// first place; without `ids` its content is still grouped.
fn groups(d: &Document, ids: Option<&[NodeId]>) -> Vec<Group> {
    let mut out: Vec<Group> = vec![];
    let mut index: BTreeMap<(String, String, bool), usize> = BTreeMap::new();
    // Looked up for every linked node: a set, not the list (which may repeat an id many times).
    let wanted: Option<std::collections::HashSet<NodeId>> = ids.map(|ids| ids.iter().copied().collect());
    let mut add = |link: &LinkInfo, key: &str, document: Option<Shown>, id: Option<NodeId>| {
        let i = *index.entry((link.path.clone(), key.to_string(), document.is_some())).or_insert_with(|| {
            let plate_files = match document {
                None => d.images.get(key).map(ImageBlob::plate_files).unwrap_or_default(),
                Some(_) => vec![],
            };
            out.push(Group { link: link.clone(), key: key.to_string(), ids: vec![], document, plate_files });
            out.len() - 1
        });
        if let (Some(id), Some(g)) = (id, out.get_mut(i)) {
            g.ids.push(id);
        }
    };
    let mut seen = std::collections::HashSet::new();
    d.visit_stored(|n, _| {
        let Some((l, key, document)) = linked(n) else { return };
        // Id 0 names no object.
        let first = n.id.0 != 0 && seen.insert(n.id);
        if first && wanted.as_ref().is_none_or(|ids| ids.contains(&n.id)) {
            add(l, key, document, Some(n.id));
        } else if ids.is_none() {
            add(l, key, document, None);
        }
    });
    out
}

/// Stored object `id` of `d` ([`Document::stored_node`]).
fn stored(d: &Document, id: NodeId) -> Option<&Node> {
    d.stored_node(id).map(|(n, _)| &**n)
}

/// Of `ids`, the objects of `d` outside the layer tree (where [`Document::node`] finds none).
fn nested_ids(d: &Document, ids: &[NodeId]) -> std::collections::HashSet<NodeId> {
    ids.iter().copied().filter(|id| d.node(*id).is_none()).collect()
}

/// `links.list`'s `owner` of an object stored at `place` of `d`, when it is not in the layer tree
/// itself: `{kind: layers|symbol|graphDesign|pattern, name?, maskOf?}`.
fn owner_json(d: &Document, place: StoredPlace) -> Option<Value> {
    if place.in_layers() {
        return None;
    }
    let mut owner = match place.root {
        StoredRoot::Layer(_) => json!({ "kind": "layers" }),
        StoredRoot::Symbol(i) => json!({ "kind": "symbol", "name": d.symbols.get(i).map(|s| s.name.as_str()) }),
        StoredRoot::GraphDesign(i) => json!({ "kind": "graphDesign", "name": d.graph_designs.get(i).map(|g| g.name.as_str()) }),
        StoredRoot::Pattern { pattern, .. } => json!({ "kind": "pattern", "name": d.patterns.get(pattern).map(|p| p.name.as_str()) }),
    };
    if let Some(m) = place.mask_of {
        owner["maskOf"] = json!(m.0);
    }
    Some(owner)
}

/// A linked file where it was found.
struct Found {
    path: String,
    size: u64,
    modified: Option<u64>,
    /// Read when telling whether it changed needed them.
    bytes: Option<Vec<u8>>,
    /// `path` relative to the folder of [`LinkBase::relative`], when [`locate`] had that file.
    relative: Option<Option<String>>,
}

/// The files a document's links are looked for from: `doc`, whose folder a link's path is followed
/// from and its file is looked for by name in, and `relative`, the file its links' stored relative
/// paths name places from (the file the document was read from).
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkBase<'a> {
    pub doc: Option<&'a str>,
    pub relative: Option<&'a str>,
}

impl<'a> LinkBase<'a> {
    /// Both the file at `path`.
    pub fn file(path: Option<&'a str>) -> Self {
        Self { doc: path, relative: path }
    }

    /// The open document `st`'s. One with no file of its own looks beside the file it was read from.
    pub(crate) fn of(st: &'a crate::DocState) -> Self {
        let (path, from) = (st.path.as_deref(), st.links_from.as_deref());
        Self { doc: path.or(from), relative: from.or(path) }
    }
}

impl Found {
    /// The file's bytes (read now unless already read).
    fn read(&mut self) -> Result<Vec<u8>> {
        match self.bytes.take() {
            Some(b) => Ok(b),
            None => read_file(&self.path),
        }
    }

    /// `base` pointing at this file as read now (`bytes`).
    fn link(&self, bytes: &[u8], base: &LinkInfo) -> LinkInfo {
        self.link_hashed(hash_bytes(bytes), base)
    }

    /// [`Self::link`] with the bytes' hash already taken.
    fn link_hashed(&self, hash: String, base: &LinkInfo) -> LinkInfo {
        LinkInfo { size: Some(self.size), modified: self.modified, hash: Some(hash), ..self.moved(base) }
    }

    /// `base` pointing at this file. A file found away from `base`'s path takes its relative path
    /// from where it was found.
    fn moved(&self, base: &LinkInfo) -> LinkInfo {
        let relative = match &self.relative {
            Some(found) if self.path != base.path => found.clone(),
            _ => base.relative.clone(),
        };
        LinkInfo { path: self.path.clone(), relative, ..base.clone() }
    }
}

/// The link to the file at `path` whose bytes were just read.
pub(crate) fn link_info(path: &str, bytes: &[u8]) -> LinkInfo {
    let path = absolute_path(path);
    LinkInfo {
        size: Some(bytes.len() as u64),
        modified: file_stamp(&path).and_then(|(_, m)| m),
        hash: Some(hash_bytes(bytes)),
        ..LinkInfo::new(path)
    }
}

/// The file at `path` when there is one.
fn found_at(path: String) -> Option<Found> {
    let (size, modified) = file_stamp(&path)?;
    Some(Found { path, size, modified, bytes: None, relative: None })
}

/// Where the file of `link` (of a document looking from `base`) is: at its path, else at its
/// relative path from the folder of [`LinkBase::relative`], then from the document's folder, else
/// by name in the document's folder, then in that of [`LinkBase::relative`]. Only paths
/// [`fileio::document_path`] follows from the document's own file are looked at.
fn locate(link: &LinkInfo, base: LinkBase) -> Option<Found> {
    let (relative, other) = (link.relative.as_deref(), base.doc != base.relative);
    let stored = [
        (Some(link.path.as_str()), base.doc),
        (relative, base.relative),
        (relative.filter(|_| other), base.doc),
        (Some(link.name()), base.doc),
        (other.then_some(link.name()), base.relative),
    ];
    let mut f = stored.into_iter().filter_map(|(p, from)| follow_from(p?, from, base.doc, fileio::document_path)).find_map(found_at)?;
    f.relative = base.relative.map(|from| {
        let from = absolute_path(from);
        Path::new(&from).parent().and_then(|dir| relative_path(Path::new(&f.path), dir))
    });
    Some(f)
}

/// The place `stored` names from the file `from`, as `resolve` ([`fileio::document_path`]) follows
/// it, when it also follows that place from the document's own file `doc`: a network share the
/// document is no longer on is refused.
pub(crate) fn follow_from(
    stored: &str,
    from: Option<&str>,
    doc: Option<&str>,
    resolve: impl Fn(&str, Option<&str>) -> std::result::Result<String, craft_docpath::Refused>,
) -> Option<String> {
    let path = resolve(stored, from).ok()?;
    if from == doc { Some(path) } else { resolve(&path, doc).ok() }
}

/// Why the path of `link` (stored in the document saved at `doc_path`) isn't followed, when it
/// names another computer, a device or a URL.
fn refused(link: &LinkInfo, doc_path: Option<&str>) -> Option<craft_docpath::Refused> {
    fileio::document_path(&link.path, doc_path).err().filter(|r| *r != craft_docpath::Refused::Invalid)
}

/// Where the file of group `g` (of a document looking from `base`) is and whether it still has
/// the content shown. Its bytes are read only when its size or modification time changed. An image
/// placed from a DCS file is modified when one of its plate files of their own changed.
fn probe(g: &Group, base: LinkBase) -> (Status, Option<Found>) {
    let Some(mut f) = locate(&g.link, base) else { return (Status::Missing, None) };
    let l = &g.link;
    if !g.plate_files.is_empty() && super::place::dcs::plate_files_changed(&g.plate_files, &f.path) {
        return (Status::Modified, Some(f));
    }
    if l.size == Some(f.size) && l.modified.is_some() && l.modified == f.modified {
        return (Status::Ok, Some(f));
    }
    let Ok(bytes) = f.read() else { return (Status::Missing, None) };
    let same = match &l.hash {
        Some(h) => *h == hash_bytes(&bytes),
        // Linked before links kept a hash: the image the file decodes to.
        None => g.document.is_none() && super::place::dcs::raster(&bytes, Some(&f.path)).is_ok_and(|img| img.key == g.key),
    };
    f.bytes = Some(bytes);
    (if same { Status::Ok } else { Status::Modified }, Some(f))
}

/// Is image `key` of `d` showing its preview (its file not read)?
fn previewing(d: &Document, key: &str) -> bool {
    d.images.get(key).is_none_or(ImageBlob::is_proxy)
}

/// A linked raster image whose file is an EPS (Preferences › File Handling › Use Low Resolution
/// Proxy for Linked EPS).
fn linked_eps(link: &LinkInfo) -> bool {
    fileio::format_for_name(link.name()).is_some_and(|f| f.id == "eps")
}

/// With `low_res_proxy_eps`, a linked EPS image that already shows its saved preview keeps it on
/// open instead of reading the file again (Update Links still reads modified files).
fn keep_eps_preview(low_res_proxy_eps: bool, g: &Group, d: &Document) -> bool {
    low_res_proxy_eps && g.document.is_none() && linked_eps(&g.link) && previewing(d, &g.key)
}

/// Is the linked file `f` found a DCS file? Only the start of the file is read, unless a binary EPS
/// header puts its PostScript further on; a DCS file read in full stays in `f`. A file that can't
/// be read is not one.
fn is_dcs_file(f: &mut Found) -> bool {
    use super::place::dcs;
    if let Some(b) = &f.bytes {
        return dcs::is_dcs(b);
    }
    let Ok(head) = fileio::read_file_head(&f.path, dcs::HEAD_BYTES) else { return false };
    if let Some(is) = dcs::head_is_dcs(&head, f.size) {
        return is;
    }
    let Ok(b) = f.read() else { return false };
    let is = dcs::is_dcs(&b);
    if is {
        f.bytes = Some(b);
    }
    is
}

/// Does group `g` of `d` show something other than its file: an image its preview, a placed
/// document nothing (its file's bytes are gone)?
fn stale(d: &Document, g: &Group) -> bool {
    match g.document {
        None => previewing(d, &g.key),
        Some(_) => !d.images.contains_key(&g.key),
    }
}

/// A linked file of a document (File → Package): its path, name and bytes.
pub(crate) struct LinkedFile {
    pub path: String,
    pub name: String,
    /// The file actually read, which may be next to the current document after a move.
    /// Nested links must be resolved against this folder, not the obsolete original path.
    pub found_path: Option<String>,
    /// Read from where the file is found, else the pixels the document holds when they are the
    /// file's own; `None` when neither.
    pub bytes: Option<Vec<u8>>,
}

/// Every file the linked images of `d` (looking from `base`) show, in the layers, symbols and
/// patterns, once each.
pub(crate) fn linked_files(d: &Document, base: LinkBase) -> Vec<LinkedFile> {
    let mut seen = std::collections::BTreeSet::new();
    groups(d, None)
        .into_iter()
        .filter(|g| seen.insert(g.link.path.clone()))
        .map(|g| {
            let named = fileio::format_for_name(g.link.name()).or_else(|| fileio::readable_format_for_name(g.link.name()));
            let own = |b: &&ImageBlob| !b.is_proxy() && named.is_some_and(|f| f.mime == b.mime);
            let found = locate(&g.link, base);
            let found_path = found.as_ref().map(|f| f.path.clone());
            let bytes = found.and_then(|mut f| f.read().ok()).or_else(|| d.images.get(&g.key).filter(own).map(|b| b.bytes.to_vec()));
            LinkedFile { path: g.link.path.clone(), name: g.link.name().to_string(), found_path, bytes }
        })
        .collect()
}

/// `d` for output: linked raster images that still show only their saved preview read their files
/// again (Preferences › Use Low Resolution Proxy for Linked EPS keeps the preview on screen only).
pub(crate) fn full_linked_images<'a>(d: &'a Document, base: LinkBase) -> (Cow<'a, Document>, Vec<String>) {
    let mut keys = BTreeSet::new();
    d.visit_images(|_, im| {
        if im.link.is_some() && d.images.get(&im.key).is_some_and(ImageBlob::is_proxy) {
            keys.insert(im.key.clone());
        }
    });
    if keys.is_empty() {
        return (Cow::Borrowed(d), vec![]);
    }
    let mut out = d.clone();
    let mut previews = BTreeSet::new();
    let all = groups(d, None);
    for key in keys {
        let Some(g) = all.iter().find(|g| g.document.is_none() && g.key == key) else { continue };
        let Some(mut f) = locate(&g.link, base) else {
            previews.insert(key);
            continue;
        };
        let Some(bytes) = f.read().ok() else {
            previews.insert(key);
            continue;
        };
        let Ok(img) = super::place::dcs::raster(&bytes, Some(&f.path)) else {
            previews.insert(key);
            continue;
        };
        let preview = d.images.get(&key).map(|b| b.bytes.clone());
        // A file read as it is keeps its own type; one turned upright by its orientation is the PNG, or CMYK TIFF, it became.
        let mime = match fileio::format_for_name(g.link.name()) {
            Some(f) if img.blob.bytes.as_slice() == bytes.as_slice() => f.mime.to_string(),
            _ => img.blob.mime.clone(),
        };
        out.images.insert(key, ImageBlob { mime, bytes: img.blob.bytes, proxy: preview, file_font: Default::default() });
    }
    let mut warnings = vec![];
    match previews.len() {
        0 => {}
        1 => warnings.push("1 linked image output as its preview: its file can't be found or read (relink or update it)".to_string()),
        n => warnings.push(format!("{n} linked images output as their previews: their files can't be found or read (relink or update them)")),
    }
    (Cow::Owned(out), warnings)
}

/// The linked files of the open document `st` that `links.findFiles` looks for, each with the
/// stored objects that show it: the files of objects `ids` whatever their status, else the files
/// that can't be found.
pub(crate) fn files_to_find(st: &crate::DocState, ids: Option<&[NodeId]>) -> Vec<(LinkInfo, Vec<NodeId>)> {
    groups(&st.doc, ids)
        .into_iter()
        .filter(|g| !g.ids.is_empty() && (ids.is_some() || probe(g, LinkBase::of(st)).0 == Status::Missing))
        .map(|g| (g.link, g.ids))
        .collect()
}

// ---------- reading files into the document ----------

/// Keep `full` (a file's pixels) as image `key` of `d`, unless pixels are there already; a
/// preview there is kept, and a linked image gets the one `full` carries ([`readable`] makes it
/// when the file is read). Nothing is decoded here.
pub(crate) fn store_image(d: &mut Document, key: &str, full: ImageBlob, linked: bool) {
    let made = full.proxy.clone().filter(|_| linked);
    let blob = match d.images.get(key) {
        Some(b) if !b.is_proxy() => ImageBlob { proxy: b.proxy.clone().or(made), ..b.clone() },
        old => ImageBlob { proxy: old.and_then(|b| b.proxy.clone()).or(made), ..full },
    };
    d.images.insert(key.to_string(), blob);
}

/// Whether `bytes` are a PNG, JPEG, GIF or WebP file: the formats [`fileio::raster_image`] keeps
/// as they are after reading only their header.
pub(crate) fn kept_unread(bytes: &[u8]) -> bool {
    use image::ImageFormat as F;
    matches!(image::guess_format(bytes), Ok(F::Png | F::Jpeg | F::Gif | F::WebP))
}

/// `blob`, an image [`fileio::raster_image`] read from a file, ready to keep. When `unread` (the
/// file is one [`kept_unread`] names), its pixels are decoded once to check that they can be read,
/// and a `linked` image's preview is made from that decode ([`ImageBlob::checked_with_proxy`]);
/// otherwise a `linked` image gets its preview ([`ImageBlob::with_proxy`]). An error when the
/// pixels can't be read.
pub(crate) fn readable(blob: ImageBlob, unread: bool, linked: bool) -> Result<ImageBlob> {
    if !unread {
        return Ok(if linked { blob.with_proxy() } else { blob });
    }
    blob.checked_with_proxy(linked).ok_or_else(|| EngineError::Other("the image's pixels can't be read".into()))
}

/// [`readable`], and the pixels it decoded are kept in the image store the renderers share
/// ([`vectorcraft_render::keep_decoded_image`]), so the first frame that draws the image finds
/// them there and does not decode the file again.
pub(crate) fn readable_and_kept(blob: ImageBlob, unread: bool, linked: bool) -> Result<ImageBlob> {
    if !unread {
        return readable(blob, unread, linked);
    }
    let (blob, pixels) = blob.checked_with_pixels(linked).ok_or_else(|| EngineError::Other("the image's pixels can't be read".into()))?;
    if let Some(pixels) = pixels {
        vectorcraft_render::keep_decoded_image(&blob, pixels);
    }
    Ok(blob)
}

/// The linked image file `bytes` (read from `path`, when there is one), read with its preview: a
/// DCS file's plates, else its image ([`super::place::dcs::raster`]); an error when its pixels
/// can't be read ([`readable`]).
fn linked_image(bytes: &[u8], path: Option<&str>) -> Result<RasterImage> {
    let mut img = super::place::dcs::raster(bytes, path)?;
    img.blob = readable(img.blob, kept_unread(bytes), true)?;
    Ok(img)
}

/// The file read for a link: the link to it and its image.
type FileRead = (LinkInfo, RasterImage);

/// The images read in one operation, by the path of the file and its content hash: groups whose
/// image keys differ but whose file is the same share one decoded image (and its encoded bytes)
/// instead of each keeping a copy.
type ImageReads = BTreeMap<String, (String, RasterImage)>;

/// The file at `f` read for a link like `base` (its image decoded, or the one `reads` holds for
/// the same file content).
fn read_image(f: &mut Found, base: &LinkInfo, reads: &mut ImageReads) -> Result<FileRead> {
    let bytes = f.read()?;
    let hash = hash_bytes(&bytes);
    let img = match reads.get(&f.path) {
        Some((h, img)) if *h == hash => img.clone(),
        _ => {
            let img = linked_image(&bytes, Some(&f.path))?;
            reads.insert(f.path.clone(), (hash.clone(), img.clone()));
            img
        }
    };
    Ok((f.link_hashed(hash, base), img))
}

/// A linked file read for the objects that show it.
enum Read {
    Image(FileRead),
    /// A VectorCraft document or a PDF or `.ai` file read for placed documents: the link to it and
    /// what they show.
    Document(LinkInfo, super::place::document::Source),
}

/// The file at `f` read for group `g`: its image, or the artboard or page `g` shows.
fn read_for(f: &mut Found, g: &Group, cmd: &str, reads: &mut ImageReads) -> Result<Read> {
    let Some((bounding, page_box)) = g.document else { return read_image(f, &g.link, reads).map(Read::Image) };
    let bytes = f.read()?;
    let page = g.link.page.unwrap_or(1);
    let src = super::place::document::source(&f.path, &bytes, Some(&f.path), page, bounding, page_box, cmd)?;
    Ok(Read::Document(f.link(&bytes, &g.link), src))
}

/// Objects `ids` show what was read (see [`install`], [`install_document`]), and with `also`, the
/// objects outside the layer tree that show group `also`'s content.
fn install_read(d: &mut Document, ids: &[NodeId], read: Read, force: Option<Preserve>, also: Option<&Group>) -> Result<()> {
    let also = also.map(|g| (g.link.path.as_str(), g.key.as_str()));
    match read {
        Read::Image(r) => install(d, ids, r, force, also),
        Read::Document(link, src) => install_document(d, ids, &link, &src, force, also),
    }
}

/// Is `n` a linked image (or, with `document`, a placed document) of the file at `path` showing
/// content `key`?
fn shows(n: &Node, (path, key): (&str, &str), document: bool) -> bool {
    match &n.kind {
        NodeKind::Image(im) => !document && im.key == key && im.link.as_ref().is_some_and(|l| l.path == path),
        NodeKind::PlacedDocument(p) => document && p.key == key && p.link.path == path,
        _ => false,
    }
}

/// The clipping path of `clip`: its room as a rectangle in its frame.
fn clip_shape((frame, room): Clip) -> PathData {
    shapes::rectangle(Rect::new(0.0, 0.0, room.0, room.1)).transformed(frame)
}

/// What happens to the clip of an object a link update remakes.
enum ClipTo {
    /// It stays as it is.
    Keep,
    /// It is clipped to this path (Placement Options' clip): the clipping path of the clip group
    /// holding just it changes, else it goes into a new clip group.
    Path(PathData),
    /// It is clipped to this path, the clipping path its DCS file draws it through: as [`ClipTo::Path`],
    /// except that a plain group holding just it (as [`ClipTo::Off`] leaves one) becomes a clip group
    /// again.
    File(PathData),
    /// It goes into a new clip group of this path, inside the clip group that holds it.
    Nest(PathData),
    /// The clipping path of the clip group holding just it goes (a DCS file's clipping path that
    /// its new file lacks): the group stays as a plain group of it, with its id, name, visibility
    /// and transparency.
    Off,
}

/// The clipping path, with its fill rule, of the clip group holding just object `id` of `d`, when
/// one holds it.
fn clip_path_of(d: &Document, id: NodeId) -> Option<(&PathData, FillRule)> {
    let g = d.node(d.parent_of(id)?)?;
    match &g.kind {
        NodeKind::Group { children, clip: true } => match &children[..] {
            [c, o] if o.id == id => match &c.kind {
                NodeKind::Path { path, rule, clipping: true, .. } => Some((path, *rule)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Do `a` and `b` have the same subpaths, each closed or open alike, and the same anchors, each
/// within a thousandth of a point?
fn same_path(a: &PathData, b: &PathData) -> bool {
    let near = |p: vectorcraft_geom::Point, q: vectorcraft_geom::Point| p.distance(q) < 1e-3;
    a.anchor_count() == b.anchor_count()
        && a.subpaths.len() == b.subpaths.len()
        && a.subpaths.iter().zip(&b.subpaths).all(|(x, y)| x.closed == y.closed)
        && a.anchors()
            .zip(b.anchors())
            .all(|((si, ai, x), (sj, aj, y))| si == sj && ai == aj && near(x.p, y.p) && near(x.h_in, y.h_in) && near(x.h_out, y.h_out))
}

/// The clip an image placed with `old_xf` takes when it shows `img`, read from a file, placed with
/// `new_xf`: Placement Options' clip `clip` when they clip it, else the clipping path a DCS file
/// draws `img` through. When neither clips it, a clip group holding just it whose clipping path
/// `held` is its old DCS file's (`old_clip`, in the old pixels, under the nonzero rule its clip
/// groups get) goes.
fn clip_to(
    old_xf: Affine,
    old_clip: Option<&PathData>,
    clip: Option<Clip>,
    img: &RasterImage,
    new_xf: Affine,
    held: Option<(&PathData, FillRule)>,
) -> ClipTo {
    if let Some(clip) = clip {
        return ClipTo::Path(clip_shape(clip));
    }
    // Whether the clip group holding just the image is the old DCS file's: a clipping path whose
    // fill rule was changed in the document is not.
    let file_held = match (old_clip, held) {
        (Some(old), Some((held, rule))) => rule == FillRule::NonZero && same_path(&old.transformed(old_xf), held),
        _ => false,
    };
    match img.clip.as_ref().map(|path| path.transformed(new_xf)) {
        // A clipping mask made in the document stays, with the file's clip group inside it.
        Some(path) if held.is_some() && !file_held => ClipTo::Nest(path),
        Some(path) => ClipTo::File(path),
        None if file_held => ClipTo::Off,
        None => ClipTo::Keep,
    }
}

/// The object an object becomes, and what happens to its clip, given the clipping path of the clip
/// group holding just it ([`ClipTo`]).
type Remake<'a> = dyn FnMut(&Node, Option<(&PathData, FillRule)>) -> (Node, ClipTo) + 'a;

/// Remakes the linked objects outside the layer tree (in symbols, graph designs, patterns and
/// opacity masks, where [`Document::node`] does not find them) that `pick` holds for.
struct Nested<'a> {
    pick: &'a dyn Fn(&Node) -> bool,
    make: &'a mut Remake<'a>,
    /// The next node id for a clip group (taken from the document after the walk).
    next: u64,
}

impl Nested<'_> {
    /// Every root of `d`: the layers (whose own objects are left alone), then the art of symbols,
    /// graph designs and patterns.
    fn run(mut self, d: &mut Document) {
        let first = d.peek_next_id();
        self.next = first;
        for l in d.layers.iter_mut() {
            self.update(l, false);
        }
        for root in d.symbols.iter_mut().map(|s| &mut s.art).chain(d.graph_designs.iter_mut().map(|g| &mut g.art)) {
            self.root(root);
        }
        for root in d.patterns.iter_mut().flat_map(|p| &mut p.art) {
            self.root(root);
        }
        for _ in first..self.next {
            d.alloc_id();
        }
    }

    /// The art at a root outside the layer tree (a symbol's, graph design's or pattern's art, or
    /// mask art).
    fn root(&mut self, n: &mut Arc<Node>) {
        if !(self.pick)(n) {
            return self.update(n, true);
        }
        let (new, clip) = (self.make)(n, None);
        let mut new = Arc::new(new);
        self.update(&mut new, true);
        *n = match clip {
            ClipTo::Path(path) | ClipTo::File(path) | ClipTo::Nest(path) => Arc::new(self.clip_group(new, path)),
            ClipTo::Keep | ClipTo::Off => new,
        };
    }

    /// `n` (outside the layer tree when `nested`) with the picked objects under it remade; it stays
    /// the same node when none is there, and only the nodes on the way to one are copied.
    fn update(&mut self, n: &mut Arc<Node>, nested: bool) {
        let mask = n.extras().mask.as_ref().and_then(|m| {
            let mut art = Arc::clone(&m.art);
            self.root(&mut art);
            (!Arc::ptr_eq(&art, &m.art)).then_some(art)
        });
        let mut slots: Vec<(usize, Arc<Node>)> = vec![];
        // The object a clip group that loses its clipping path holds alone.
        let mut declipped = None;
        // The clipping path and object of a plain group holding just the object, which becomes a
        // clip group again.
        let mut reclipped = None;
        if let Some(children) = n.children() {
            // A clip group holding just the object: its clipping path changes.
            let clip_path = match (&n.kind, &children[..]) {
                (NodeKind::Group { clip: true, .. }, [c, _]) if matches!(c.kind, NodeKind::Path { clipping: true, .. }) => Some(c),
                _ => None,
            };
            let plain = matches!((&n.kind, &children[..]), (NodeKind::Group { clip: false, .. }, [_]));
            for (i, c) in children.iter().enumerate() {
                if !(nested && (self.pick)(c)) {
                    let mut new = Arc::clone(c);
                    self.update(&mut new, nested);
                    if !Arc::ptr_eq(&new, c) {
                        slots.push((i, new));
                    }
                    continue;
                }
                let held = clip_path.filter(|_| i == 1).and_then(|p| match &p.kind {
                    NodeKind::Path { path, rule, .. } => Some((path, *rule)),
                    _ => None,
                });
                let (new, clip) = (self.make)(c, held);
                let mut new = Arc::new(new);
                self.update(&mut new, true);
                match (clip, clip_path) {
                    (ClipTo::Keep, _) => slots.push((i, new)),
                    (ClipTo::Path(clip) | ClipTo::File(clip), Some(path)) if i == 1 => {
                        let mut path = Node::clone(path);
                        if let NodeKind::Path { path: p, .. } = &mut path.kind {
                            *p = clip;
                        }
                        slots.extend([(0, Arc::new(path)), (1, new)]);
                    }
                    (ClipTo::File(clip), None) if plain => reclipped = Some((clip, new)),
                    (ClipTo::Path(clip) | ClipTo::File(clip) | ClipTo::Nest(clip), _) => {
                        let group = self.clip_group(new, clip);
                        slots.push((i, Arc::new(group)));
                    }
                    (ClipTo::Off, Some(_)) if i == 1 => declipped = Some(new),
                    (ClipTo::Off, _) => slots.push((i, new)),
                }
            }
        }
        if mask.is_none() && slots.is_empty() && declipped.is_none() && reclipped.is_none() {
            return;
        }
        let reclipped = reclipped.map(|(clip, object)| (Arc::new(self.clipping_path(clip)), object));
        let n = Arc::make_mut(n);
        if let (Some(art), Some(m)) = (mask, n.extras_if_any().and_then(|e| e.mask.as_mut())) {
            m.art = art;
        }
        // A plain group of just the object that becomes a clip group again.
        if let Some((path, object)) = reclipped {
            if let NodeKind::Group { children, clip } = &mut n.kind {
                (*children, *clip) = (vec![path, object], true);
            }
            return;
        }
        // A clip group that stays as a plain group of just the object.
        if let Some(object) = declipped {
            if let NodeKind::Group { children, clip } = &mut n.kind {
                (*children, *clip) = (vec![object], false);
            }
            return;
        }
        if let Some(children) = n.children_mut() {
            for (i, c) in slots {
                if let Some(slot) = children.get_mut(i) {
                    *slot = c;
                }
            }
        }
    }

    /// A clip group of `object` clipped to `clip`, as [`clip_image_to`] makes one in the layers.
    fn clip_group(&mut self, object: Arc<Node>, clip: PathData) -> Node {
        let path = self.clipping_path(clip);
        let group_id = NodeId(self.next);
        self.next += 1;
        Node::new(group_id, NodeKind::Group { children: vec![Arc::new(path), object], clip: true })
    }

    /// A clipping path of `clip`, with the next node id.
    fn clipping_path(&mut self, clip: PathData) -> Node {
        let id = NodeId(self.next);
        self.next += 1;
        clipping_path(id, clip)
    }
}

/// Placed documents `ids` (in the layers or outside them) show `src`, read from the file `link`
/// points at, each placed as its Placement Options say (`force`: as that Preserve choice says,
/// whatever they say); with `also` (a file's path and the content shown), so do the placed
/// documents outside the layer tree that show that content.
fn install_document(
    d: &mut Document,
    ids: &[NodeId],
    link: &LinkInfo,
    src: &super::place::document::Source,
    force: Option<Preserve>,
    also: Option<(&str, &str)>,
) -> Result<()> {
    let mut clips = vec![];
    let (w, h) = (src.frame.width(), src.frame.height());
    let placed = |p: &PlacedDocument| {
        let o = PlacementOptions { preserve: force.unwrap_or(p.placement.preserve), ..p.placement };
        placed_xf(p.xf, (p.width, p.height), (1.0, 1.0), (w, h), (1.0, 1.0), o)
    };
    let nested = nested_ids(d, ids);
    if also.is_some() || !nested.is_empty() {
        let pick =
            |n: &Node| matches!(n.kind, NodeKind::PlacedDocument(_)) && (nested.contains(&n.id) || also.is_some_and(|also| shows(n, also, true)));
        let mut make = |n: &Node, _: Option<(&PathData, FillRule)>| {
            let mut n = n.clone();
            let mut clip = ClipTo::Keep;
            if let NodeKind::PlacedDocument(p) = &mut n.kind {
                let (xf, c) = placed(p);
                (p.key, p.width, p.height, p.xf, p.link) = (src.key.clone(), w, h, xf, link.clone());
                clip = c.map_or(ClipTo::Keep, |c| ClipTo::Path(clip_shape(c)));
            }
            (n, clip)
        };
        Nested { pick: &pick, make: &mut make, next: 0 }.run(d);
    }
    for id in ids {
        let Some(NodeKind::PlacedDocument(p)) = d.node(*id).map(|n| &n.kind) else { continue };
        let (xf, clip) = placed(p);
        if let Some(n) = d.node_mut(*id)
            && let NodeKind::PlacedDocument(p) = &mut n.kind
        {
            let p: &mut PlacedDocument = p;
            (p.key, p.width, p.height, p.xf, p.link) = (src.key.clone(), w, h, xf, link.clone());
        }
        clips.extend(clip.map(|c| (*id, c)));
    }
    super::place::document::store(d, src);
    clips.into_iter().try_for_each(|(id, clip)| clip_image(d, id, clip))
}

/// Points per pixel of the file image `key` of `d` shows, at the resolution it declares (72 ppi
/// when it declares none, or when only its preview is loaded).
fn file_pt(d: &Document, key: &str) -> (f64, f64) {
    super::place::pt_per_px(file_ppi(d, key))
}

/// The resolution the file image `key` of `d` shows declares (none while only its preview is
/// loaded).
fn file_ppi(d: &Document, key: &str) -> Option<(f64, f64)> {
    d.images.get(key).filter(|b| !b.is_proxy()).and_then(|b| fileio::ppi::resolution(&b.bytes))
}

/// Where the clip of an image goes: the old bounds' frame (bounds space to the document) and size.
type Clip = (Affine, (f64, f64));

/// The transform an image placed with `xf` showing `old` pixels (`old_pt`: points per pixel at
/// 100%) gives a new file of `new` pixels (`new_pt`), as Placement Options `o` say; and the clip
/// when `o.clip` and the new art overflows the old bounds. (A placed document's "pixels" are points,
/// one point each.)
fn placed_xf(xf: Affine, old: (f64, f64), old_pt: (f64, f64), new: (f64, f64), new_pt: (f64, f64), o: PlacementOptions) -> (Affine, Option<Clip>) {
    let (ow, oh, nw, nh) = (old.0, old.1, new.0.max(1e-9), new.1.max(1e-9));
    let [a, b, c, d, ..] = xf.as_coeffs();
    // The bounds' frame: the transform without its scale (rotation, reflection and position).
    let (su, sv) = (a.hypot(b), c.hypot(d));
    if o.preserve == Preserve::Bounds || su < 1e-12 || sv < 1e-12 {
        return (xf * Affine::scale_non_uniform(ow / nw, oh / nh), None);
    }
    let (frame, room) = match o.preserve {
        Preserve::FileDimensions => {
            let r = xf.transform_rect_bbox(Rect::new(0.0, 0.0, ow, oh));
            (Affine::translate(r.origin().to_vec2()), (r.width(), r.height()))
        }
        _ => (xf * Affine::scale_non_uniform(1.0 / su, 1.0 / sv), (ow * su, oh * sv)),
    };
    // The new file's size at 100% (pt), then as placed.
    let natural = (nw * new_pt.0, nh * new_pt.1);
    let size = match o.preserve {
        Preserve::Transforms => (natural.0 * su / old_pt.0, natural.1 * sv / old_pt.1),
        Preserve::Fit | Preserve::Fill => {
            let (kx, ky) = (room.0 / natural.0, room.1 / natural.1);
            let k = if o.preserve == Preserve::Fit { kx.min(ky) } else { kx.max(ky) };
            (natural.0 * k, natural.1 * k)
        }
        _ => natural,
    };
    let (fx, fy) = o.align.fractions();
    let at = Affine::translate(((room.0 - size.0) * fx, (room.1 - size.1) * fy));
    let over = size.0 > room.0 + 1e-6 || size.1 > room.1 + 1e-6;
    (frame * at * Affine::scale_non_uniform(size.0 / nw, size.1 / nh), (o.clip && over).then_some((frame, room)))
}

/// The clipping path of the clip group of `d` holding just object `id` becomes `path`; false when
/// no such clip group holds it.
fn set_clip_path(d: &mut Document, id: NodeId, path: PathData) -> bool {
    let clip_path = d.parent_of(id).and_then(|p| match &d.node(p)?.kind {
        NodeKind::Group { children, clip: true } => match &children[..] {
            [c, im] if im.id == id && matches!(c.kind, NodeKind::Path { clipping: true, .. }) => Some(c.id),
            _ => None,
        },
        _ => None,
    });
    let Some(c) = clip_path else { return false };
    if let Some(Node { kind: NodeKind::Path { path: p, .. }, .. }) = d.node_mut(c) {
        *p = path;
    }
    true
}

/// Clip image `id` of `d` to `room` in `frame`: the clipping path of the clip group holding just
/// it changes, else the image goes into a new clip group.
fn clip_image(d: &mut Document, id: NodeId, clip: Clip) -> Result<()> {
    clip_image_to(d, id, clip_shape(clip))
}

/// Clip object `id` of `d` to `path`: the clipping path of the clip group holding just it changes,
/// else it goes into a new clip group.
fn clip_image_to(d: &mut Document, id: NodeId, path: PathData) -> Result<()> {
    if set_clip_path(d, id, path.clone()) {
        return Ok(());
    }
    new_clip_group(d, id, path)
}

/// Clip object `id` of `d` to `path`, the clipping path its DCS file draws it through: the clipping
/// path of the clip group holding just it changes, a plain group holding just it (as [`unclip`]
/// leaves one) becomes a clip group again, else it goes into a new clip group.
fn clip_file_to(d: &mut Document, id: NodeId, path: PathData) -> Result<()> {
    if set_clip_path(d, id, path.clone()) {
        return Ok(());
    }
    let plain = d.parent_of(id).filter(|g| match d.node(*g).map(|n| &n.kind) {
        Some(NodeKind::Group { children, clip: false }) => matches!(&children[..], [o] if o.id == id),
        _ => false,
    });
    let Some(g) = plain else { return new_clip_group(d, id, path) };
    let clip = clipping_path(d.alloc_id(), path);
    d.insert(Some(g), 0, clip)?;
    if let Some(Node { kind: NodeKind::Group { clip, .. }, .. }) = d.node_mut(g) {
        *clip = true;
    }
    Ok(())
}

/// A clipping path `id` of `path`, unpainted.
fn clipping_path(id: NodeId, path: PathData) -> Node {
    let none = vectorcraft_color::Paint::None;
    let mut clip = Node::path(id, path, Appearance::basic(none.clone(), none, 0.0));
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    clip
}

/// Put object `id` of `d` into a new clip group of `path`, where it is.
fn new_clip_group(d: &mut Document, id: NodeId, path: PathData) -> Result<()> {
    let (par, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    let image = d.remove(id)?;
    let clip = clipping_path(d.alloc_id(), path);
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![std::sync::Arc::new(clip), image], clip: true });
    d.insert(par, index, group)?;
    Ok(())
}

/// Replace image `id` with the raster at `path`, keeping the object's bounds (what an image
/// variable does on `dataset.select`). The file is linked; a missing or unreadable path is an
/// error so the dataset application can skip that binding.
pub(crate) fn set_image_from_path(d: &mut Document, id: NodeId, path: &str, doc_path: Option<&str>) -> Result<()> {
    // A path stored in the document (a data set's value) is followed as its other stored paths
    // are: on this computer or the document's own share only, so choosing a data set can't make
    // the app connect to another computer.
    let path = fileio::document_path(path, doc_path).map_err(|r| EngineError::Other(format!("the image `{path}` isn't followed: {r}")))?;
    let bytes = read_file(&path)?;
    let read = (link_info(&path, &bytes), linked_image(&bytes, Some(&path))?);
    install(d, &[id], read, Some(Preserve::Bounds), None)
}

/// Image objects `ids` (in the layers or outside them) show the file read for `link` (`img`), each
/// placed as its Placement Options say (`force`: as that Preserve choice says, whatever they say);
/// with `also` (a file's path and the content shown), so do the images outside the layer tree that
/// show that content.
fn install(d: &mut Document, ids: &[NodeId], (link, img): FileRead, force: Option<Preserve>, also: Option<(&str, &str)>) -> Result<()> {
    let new_pt = super::place::pt_per_px(img.ppi);
    let mut clips = vec![];
    let nested = nested_ids(d, ids);
    if also.is_some() || !nested.is_empty() {
        let pick = |n: &Node| matches!(n.kind, NodeKind::Image(_)) && (nested.contains(&n.id) || also.is_some_and(|also| shows(n, also, false)));
        // The resolution of each content the images picked show, and the clipping path its DCS file
        // drew it through, looked up once each.
        let mut old: BTreeMap<String, ((f64, f64), Option<PathData>)> = BTreeMap::new();
        d.visit_stored(|n, _| {
            if let NodeKind::Image(im) = &n.kind
                && pick(n)
                && !old.contains_key(&im.key)
            {
                old.insert(im.key.clone(), (file_pt(d, &im.key), d.images.get(&im.key).and_then(ImageBlob::separated_clip)));
            }
        });
        let mut make = |n: &Node, held: Option<(&PathData, FillRule)>| {
            let mut n = n.clone();
            let mut clip = ClipTo::Keep;
            if let NodeKind::Image(im) = &mut n.kind {
                let o = PlacementOptions { preserve: force.unwrap_or(im.placement.preserve), ..im.placement };
                let size = |w: u32, h: u32| (w as f64, h as f64);
                let (old_pt, old_clip) = match old.get(&im.key) {
                    Some((pt, clip)) => (*pt, clip.as_ref()),
                    None => (super::place::pt_per_px(None), None),
                };
                let (xf, c) = placed_xf(im.xf, size(im.width, im.height), old_pt, size(img.width, img.height), new_pt, o);
                clip = clip_to(im.xf, old_clip, c, &img, xf, held);
                (im.key, im.width, im.height, im.xf, im.link) = (img.key.clone(), img.width, img.height, xf, Some(link.clone()));
            }
            (n, clip)
        };
        Nested { pick: &pick, make: &mut make, next: 0 }.run(d);
    }
    for id in ids {
        let Some(NodeKind::Image(im)) = d.node(*id).map(|n| &n.kind) else { continue };
        let o = PlacementOptions { preserve: force.unwrap_or(im.placement.preserve), ..im.placement };
        let size = |w: u32, h: u32| (w as f64, h as f64);
        let (xf, clip) = placed_xf(im.xf, size(im.width, im.height), file_pt(d, &im.key), size(img.width, img.height), new_pt, o);
        // Placement Options' clip, else a DCS file's clipping path, read again with its plates.
        let old_clip = d.images.get(&im.key).and_then(ImageBlob::separated_clip);
        clips.push((*id, clip_to(im.xf, old_clip.as_ref(), clip, &img, xf, clip_path_of(d, *id))));
        if let Some(n) = d.node_mut(*id)
            && let NodeKind::Image(im) = &mut n.kind
        {
            (im.key, im.width, im.height, im.xf, im.link) = (img.key.clone(), img.width, img.height, xf, Some(link.clone()));
        }
    }
    // A DCS file's spot plates print on spot swatches of their inks.
    super::place::dcs::add_spot_swatches(d, &img.blob.spot_plates());
    store_image(d, &img.key, img.blob, true);
    clips.into_iter().try_for_each(|(id, clip)| match clip {
        ClipTo::Keep => Ok(()),
        ClipTo::Path(path) => clip_image_to(d, id, path),
        ClipTo::File(path) => clip_file_to(d, id, path),
        ClipTo::Nest(path) => new_clip_group(d, id, path),
        ClipTo::Off => unclip(d, id),
    })
}

/// Take the clipping path of the clip group holding just object `id` of `d` away: the group stays
/// as a plain group of the object, with its id, name, visibility and transparency as they were.
fn unclip(d: &mut Document, id: NodeId) -> Result<()> {
    let Some((g, clip)) = d.parent_of(id).and_then(|g| match &d.node(g)?.kind {
        NodeKind::Group { children, clip: true } => match &children[..] {
            [c, o] if o.id == id && matches!(c.kind, NodeKind::Path { clipping: true, .. }) => Some((g, c.id)),
            _ => None,
        },
        _ => None,
    }) else {
        return Ok(());
    };
    d.remove(clip)?;
    if let Some(Node { kind: NodeKind::Group { clip, .. }, .. }) = d.node_mut(g) {
        *clip = false;
    }
    Ok(())
}

/// Groups (of the document saved at `doc_path`) as `{name, path, ids}` rows (those with objects in
/// the layers), with `refused`: why the path isn't followed, when it isn't. `path` is left out
/// where [`path_shown`] says so.
fn rows<'a>(gs: impl Iterator<Item = &'a Group>, doc_path: Option<&str>) -> Vec<Value> {
    gs.filter(|g| !g.ids.is_empty())
        .map(|g| {
            let mut row = json!({ "name": g.link.name(), "ids": ids_json(&g.ids) });
            note_path(&mut row, "path", &g.link.path, doc_path);
            note_refused(&mut row, &g.link, doc_path);
            row
        })
        .collect()
}

/// May a reply to the caller asking now carry the link path `path` (of the document saved at
/// `doc_path`)? Always without read roots in force; with them, only when the file it names (a
/// relative path taken from the document's folder) lies inside them.
pub(crate) fn path_shown(path: &str, doc_path: Option<&str>) -> bool {
    crate::file_access::current().is_none() || fileio::document_path(path, doc_path).is_ok_and(|p| crate::file_access::check_read(&p).is_ok())
}

/// Is the linked JavaScript file `f` (of the document saved at `doc_path`) named by its file name
/// alone in a reply to the caller asking now? Only a path [`path_shown`] does not allow; a URL is
/// always shown as it is.
pub(crate) fn script_hidden(f: &str, doc_path: Option<&str>) -> bool {
    !path_shown(f, doc_path) && !matches!(fileio::document_path(f, doc_path), Err(craft_docpath::Refused::Url))
}

/// The linked JavaScript file `f` of the open document `st` as a reply to the caller asking now
/// may name it: as it is, or by its file name alone where [`script_hidden`] says so.
pub fn shown_script<'a>(st: &crate::DocState, f: &'a str) -> Cow<'a, str> {
    if script_hidden(f, LinkBase::of(st).doc) { Cow::Owned(LinkInfo::new(f).name().to_string()) } else { Cow::Borrowed(f) }
}

/// `row[key] = path` when [`path_shown`] says so; otherwise `row` has no `key`.
pub(crate) fn note_path(row: &mut Value, key: &str, path: &str, doc_path: Option<&str>) {
    if path_shown(path, doc_path) {
        row[key] = json!(path);
    } else if let Some(o) = row.as_object_mut() {
        o.remove(key);
    }
}

/// `row["refused"]`: why the path of `link` isn't followed, when it isn't ([`refused`]).
fn note_refused(row: &mut Value, link: &LinkInfo, doc_path: Option<&str>) {
    if let Some(r) = refused(link, doc_path) {
        row["refused"] = json!(r.to_string());
    }
}

fn ids_json(ids: &[NodeId]) -> Vec<u64> {
    ids.iter().map(|id| id.0).collect()
}

/// What [`resolve`] found when a document opened.
#[derive(Default)]
pub struct Resolved {
    /// `{name, path, ids}` of files that weren't found: their images show the preview.
    pub missing: Vec<Value>,
    /// Modified files left as they were (Update Links isn't Automatically).
    pub modified: Vec<Value>,
    /// Modified files read again (Update Links: Automatically).
    pub updated: Vec<Value>,
}

impl Resolved {
    /// The `document.open` result fields.
    pub fn to_json(&self) -> Value {
        json!({ "missingLinks": self.missing, "modifiedLinks": self.modified, "updatedLinks": self.updated })
    }
}

/// The linked files of document `d`, just read and looking from `base`: those found unchanged are
/// read again (their images had the preview), modified ones are read again when `update`, links
/// found away from their path take the new path. A link nothing is known of whose image, or placed
/// document, shows a preview (a file another app wrote kept only that) was never read: its file is
/// read now, into the object's bounds, which isn't a change to report. A placed document whose
/// file's bytes are gone isn't showing a preview. Nothing here is an undo step.
pub fn resolve(d: &mut Document, base: LinkBase, update: bool, low_res_proxy_eps: bool) -> Resolved {
    let mut out = Resolved::default();
    // The image (with its preview) and hash of each file read for a preview here, by path: images
    // of other keys linking to the same file share them, rather than each reading, decoding and
    // keeping a copy of its own.
    let mut read_images: BTreeMap<String, (ImageBlob, String)> = BTreeMap::new();
    // (the path as it was, the content shown, a placed document?, the link now) of each file found
    // away from its path or read again.
    let mut moved: Vec<(String, String, bool, LinkInfo)> = vec![];
    let mut image_reads = ImageReads::new();
    for g in groups(d, None) {
        let row = rows(std::iter::once(&g), base.doc);
        let shows_preview = match g.document {
            None => previewing(d, &g.key),
            Some(_) => d.images.get(&g.key).is_some_and(ImageBlob::is_proxy),
        };
        let unread = g.link.hash.is_none() && g.link.size.is_none() && shows_preview;
        let eps_preview = keep_eps_preview(low_res_proxy_eps, &g, d);
        match probe(&g, base) {
            (Status::Missing, _) => out.missing.extend(row),
            (Status::Modified, Some(mut f)) if update || unread => {
                let force = unread.then_some(Preserve::Bounds);
                match read_for(&mut f, &g, "document.open", &mut image_reads).and_then(|read| install_read(d, &g.ids, read, force, Some(&g))) {
                    Ok(()) if unread => {}
                    Ok(()) => out.updated.extend(row),
                    Err(_) => out.missing.extend(row),
                }
            }
            (Status::Modified, _) => out.modified.extend(row),
            // A placed document keeps its file's bytes: only lost ones are read again.
            (Status::Ok, Some(mut f)) if g.document.is_some() => {
                let read = if stale(d, &g) { Some(read_for(&mut f, &g, "document.open", &mut image_reads)) } else { None };
                match read {
                    Some(Ok(read)) => {
                        if install_read(d, &g.ids, read, Some(Preserve::Bounds), Some(&g)).is_err() {
                            out.missing.extend(row);
                        }
                    }
                    Some(Err(_)) => out.missing.extend(row),
                    None => {
                        let link = match f.bytes.take() {
                            Some(b) => f.link(&b, &g.link),
                            None => f.moved(&g.link),
                        };
                        if link != g.link {
                            set_links(d, &g.ids, &link);
                            moved.push((g.link.path.clone(), g.key.clone(), true, link));
                        }
                    }
                }
            }
            (Status::Ok, Some(mut f)) => {
                let preview = previewing(d, &g.key);
                // A DCS file is read in full even with the proxy: its plates print and separate on
                // their own inks.
                let eps_preview = eps_preview && !is_dcs_file(&mut f);
                if let Some((blob, hash)) = read_images.get(&f.path).filter(|_| preview && !eps_preview).cloned() {
                    store_image(d, &g.key, blob, true);
                    let link = f.link_hashed(hash, &g.link);
                    if link != g.link {
                        set_links(d, &g.ids, &link);
                        moved.push((g.link.path.clone(), g.key.clone(), false, link));
                    }
                    continue;
                }
                // Read when the images show the preview, or when the file's details changed.
                let bytes = if (preview && !eps_preview) || f.bytes.is_some() { f.read().ok() } else { None };
                if preview && !eps_preview {
                    let Some((b, img)) = bytes.as_deref().and_then(|b| Some((b, linked_image(b, Some(&f.path)).ok()?))) else {
                        out.missing.extend(row);
                        continue;
                    };
                    let blob = img.blob;
                    read_images.insert(f.path.clone(), (blob.clone(), hash_bytes(b)));
                    store_image(d, &g.key, blob, true);
                } else if let Some(b) = d.images.get(&g.key).filter(|b| b.proxy.is_none()) {
                    // Linked before links kept a preview.
                    let b = b.clone().with_proxy();
                    d.images.insert(g.key.clone(), b);
                }
                // Found away from its path, or read again: the link follows.
                let link = match &bytes {
                    Some(b) => f.link(b, &g.link),
                    None => f.moved(&g.link),
                };
                if link != g.link {
                    set_links(d, &g.ids, &link);
                    moved.push((g.link.path.clone(), g.key.clone(), false, link));
                }
            }
            (Status::Ok, None) => {}
        }
    }
    // The objects in symbols, graph designs, patterns and opacity masks that show the same file and
    // content, whatever else their links hold, those whose id an earlier object has included.
    if !moved.is_empty() {
        let mut by_path: BTreeMap<String, Vec<(String, bool, LinkInfo)>> = BTreeMap::new();
        for (path, key, document, new) in moved {
            by_path.entry(path).or_default().push((key, document, new));
        }
        let new = |n: &Node| {
            let (l, key, document) = linked(n)?;
            let m = by_path.get(&l.path)?;
            m.iter().find(|(k, doc, _)| k == key && *doc == document.is_some()).map(|(_, _, new)| new)
        };
        let pick = |n: &Node| new(n).is_some();
        let mut make = |n: &Node, _: Option<(&PathData, FillRule)>| {
            let mut out = n.clone();
            if let (Some(link), Some(l)) = (new(n), link_mut(&mut out)) {
                *l = link.clone();
            }
            (out, ClipTo::Keep)
        };
        Nested { pick: &pick, make: &mut make, next: 0 }.run(d);
    }
    // Color masks of images whose files were read again are made for the new pixels.
    super::refresh_color_masks(None, d);
    out
}

/// The link of linked object `n` (a linked image or a placed document) to change.
fn link_mut(n: &mut Node) -> Option<&mut LinkInfo> {
    match &mut n.kind {
        NodeKind::Image(im) => im.link.as_mut(),
        NodeKind::PlacedDocument(p) => Some(&mut p.link),
        _ => None,
    }
}

/// Linked image objects (or placed documents) `ids`, in the layers or outside them, link to `link`.
fn set_links(d: &mut Document, ids: &[NodeId], link: &LinkInfo) {
    let nested = nested_ids(d, ids);
    for id in ids.iter().filter(|id| !nested.contains(id)) {
        if let Some(l) = d.node_mut(*id).and_then(link_mut) {
            *l = link.clone();
        }
    }
    if !nested.is_empty() {
        let pick = |n: &Node| nested.contains(&n.id) && linked(n).is_some();
        let mut make = |n: &Node, _: Option<(&PathData, FillRule)>| {
            let mut out = n.clone();
            if let Some(l) = link_mut(&mut out) {
                *l = link.clone();
            }
            (out, ClipTo::Keep)
        };
        Nested { pick: &pick, make: &mut make, next: 0 }.run(d);
    }
}

/// Placed documents of the document saved at `path` (document `saved` of `s`) in the other open
/// documents, in the layers, symbols, graph designs, patterns and opacity masks: read again from
/// the new file, each as one Update Links step of its document.
pub(crate) fn refresh_placed(s: &mut Session, saved: u64, path: &str) {
    let path = absolute_path(path);
    let targets: Vec<usize> = s
        .documents()
        .iter()
        .enumerate()
        .filter(|(_, st)| st.uid != saved)
        .filter(|(_, st)| {
            let mut placed = false;
            st.doc.visit_placed(|_, p| placed |= p.link.path == path);
            placed
        })
        .map(|(i, _)| i)
        .collect();
    if targets.is_empty() {
        return;
    }
    // Each document is updated as the active one, through its own profiles, without switching
    // tools.
    let back = s.active;
    for i in targets {
        s.active = Some(i);
        s.sync_front_profiles();
        // A file that can't be read leaves what its objects show: links.check reports it.
        let _ = update_where(s, None, Some(&path));
    }
    s.active = back;
    s.sync_front_profiles();
}

// ---------- relative paths ----------

/// `target` relative to folder `base` with `/` separators; `None` when they share no root (another
/// drive, or either is relative).
fn relative_path(target: &Path, base: &Path) -> Option<String> {
    let (t, b): (Vec<Component>, Vec<Component>) = (target.components().collect(), base.components().collect());
    if !matches!(t.first(), Some(Component::Prefix(_) | Component::RootDir)) {
        return None;
    }
    // Windows paths compare without case.
    let same = |x: &Component, y: &Component| if cfg!(windows) { x.as_os_str().eq_ignore_ascii_case(y.as_os_str()) } else { x == y };
    let common = t.iter().zip(&b).take_while(|(x, y)| same(x, y)).count();
    if common == 0 {
        return None;
    }
    let ups = std::iter::repeat_n(std::borrow::Cow::Borrowed(".."), b.len().saturating_sub(common));
    let rest = t.iter().skip(common).map(|c| c.as_os_str().to_string_lossy());
    Some(ups.chain(rest).collect::<Vec<std::borrow::Cow<str>>>().join("/"))
}

/// The file `link` (of a document looking from `base`) is written relative to: where [`locate`]
/// finds it, else the place its relative path names from the folder of [`LinkBase::relative`],
/// else its path.
fn link_target(link: &LinkInfo, base: LinkBase) -> PathBuf {
    if let Some(f) = locate(link, base) {
        return PathBuf::from(f.path);
    }
    match (base.relative, link.relative.as_deref()) {
        (Some(_), Some(rel)) => fileio::document_path(rel, base.relative).map_or_else(|_| PathBuf::from(&link.path), PathBuf::from),
        _ => PathBuf::from(&link.path),
    }
}

/// `d` (looking from `base`) as saved to `dest`: the links' paths relative to its folder written
/// in; `None` when they are already. A link whose file is not at its path is written relative to
/// where its file is found, else to the place its relative path names. Links in the layers,
/// symbols, graph designs, patterns and opacity masks are all set.
pub fn with_relative_paths(d: &Document, base: LinkBase, dest: &str) -> Option<Document> {
    let dest = absolute_path(dest);
    let dir = Path::new(&dest).parent()?;
    // The relative path of each link by its path and stored relative path, each file looked at once.
    let mut wanted: BTreeMap<(String, Option<String>), Option<String>> = BTreeMap::new();
    let mut look = |l: &LinkInfo| {
        wanted.entry((l.path.clone(), l.relative.clone())).or_insert_with(|| relative_path(&link_target(l, base), dir));
    };
    d.visit_images(|_, im| im.link.iter().for_each(&mut look));
    d.visit_placed(|_, p| look(&p.link));
    wanted.retain(|(_, stored), rel| rel != stored);
    if wanted.is_empty() {
        return None;
    }
    let key = |l: &LinkInfo| (l.path.clone(), l.relative.clone());
    let mut d = d.clone();
    d.update_links_where(|l| wanted.contains_key(&key(l)), |l| l.relative = wanted.get(&key(l)).cloned().flatten());
    Some(d)
}

/// Sets each link of `d`, a copy of a document looking from `base`, to the path where that
/// document finds its file, and its relative path to it as that document's links have them, so
/// the copy finds the same files. Links in the layers, symbols, graph designs, patterns and opacity
/// masks are all set.
pub(crate) fn with_found_paths(d: &mut Document, base: LinkBase) {
    // The new path and relative path of each link by what `locate` reads of it: its path, relative
    // path and name.
    type PathAndRelative = (String, Option<String>);
    let mut found: BTreeMap<PathAndRelative, Option<PathAndRelative>> = BTreeMap::new();
    let mut look = |l: &LinkInfo| {
        found.entry((l.path.clone(), l.relative.clone())).or_insert_with(|| {
            let moved = locate(l, base)?.moved(l);
            Some((moved.path, moved.relative))
        });
    };
    d.visit_images(|_, im| im.link.iter().for_each(&mut look));
    d.visit_placed(|_, p| look(&p.link));
    found.retain(|(path, _), f| f.as_ref().is_some_and(|(f, _)| f != path));
    if found.is_empty() {
        return;
    }
    d.update_links(|l| {
        if let Some(Some((path, relative))) = found.get(&(l.path.clone(), l.relative.clone())) {
            l.path = path.clone();
            l.relative = relative.clone();
        }
    });
}

/// `d`, a document saved at `doc_path`, as a reply to the caller asking now may show it: without
/// read roots in force, `d` itself; with them, a copy in which each link [`path_shown`] does not
/// allow names its file alone, without its relative path or the size and date kept of its file,
/// and each data set's image value and linked JavaScript file (`svg_scripts`) it does not allow
/// names its file alone too, while a JavaScript file given as a URL stays as it is. The folder Export
/// for Screens last wrote to is left out of the export settings when it is outside the read roots.
/// A link whose path is allowed but whose relative path leads outside the read roots loses its
/// relative path. Links in the layers, symbols, graph designs, patterns, the pattern being edited
/// as it was before, opacity masks and the art of the stored brushes are all checked. A stored
/// brush entry that does not read as a brush is left out of the copy, since its links cannot be
/// checked, and a stored brush library that is not a list becomes an empty one.
pub(crate) fn with_hidden_paths_named<'a>(d: &'a Document, base: LinkBase) -> Cow<'a, Document> {
    if crate::file_access::current().is_none() {
        return Cow::Borrowed(d);
    }
    let doc_path = base.doc;
    // A stored relative path is followed from the folder of the file the document was read from,
    // and a link is also looked for from the document's own folder: it is shown only when both
    // places it names are inside the read roots.
    let relative_hidden = |r: &str| [base.relative, base.doc].into_iter().any(|from| !path_shown(r, from));
    let mut d = d.clone();
    // The pattern being edited as it was before is checked as one more pattern, then put back.
    let original = d.pattern_edit.as_mut().and_then(|e| e.original.take());
    let edited = original.is_some();
    d.patterns.extend(original);
    // The art of each stored brush is checked as more layers, then put back.
    let stored = d.unknown.get(vectorcraft_brush::DOC_KEY).map(|v| v.as_array().cloned().unwrap_or_default());
    let mut brushes: Vec<vectorcraft_brush::Brush> =
        stored.iter().flatten().filter_map(|raw| vectorcraft_brush::Brush::from_json(raw.clone()).ok()).collect();
    let layers = d.layers.len();
    d.layers.extend(brushes.iter().flat_map(|b| b.art()).map(|n| Arc::new(n.clone())));
    d.update_links_where(
        |l| !path_shown(&l.path, base.doc) || l.relative.as_deref().is_some_and(relative_hidden),
        |l| {
            if path_shown(&l.path, base.doc) {
                l.relative = None;
            } else {
                let name = l.name().to_string();
                *l = LinkInfo { path: name, relative: None, modified: None, size: None, ..l.clone() };
            }
        },
    );
    let mut checked = d.layers.split_off(layers).into_iter();
    if edited && let (Some(def), Some(e)) = (d.patterns.pop(), d.pattern_edit.as_mut()) {
        e.original = Some(def);
    }
    if stored.is_some() {
        // Each brush is written as it reads, so fields it does not read are left out too.
        let mut kept = vec![];
        for b in &mut brushes {
            for (slot, art) in b.art_mut().into_iter().zip(&mut checked) {
                *slot = Arc::unwrap_or_clone(art);
            }
            kept.extend(serde_json::to_value(&*b).ok());
        }
        d.unknown.insert(vectorcraft_brush::DOC_KEY.into(), Value::Array(kept));
    }
    if d.export_settings.get("folder").and_then(Value::as_str).is_some_and(|f| crate::file_access::check_read(f).is_err()) {
        d.export_settings.remove("folder");
    }
    let hidden = |v: &DataValue| matches!(v, DataValue::Image(path) if !path_shown(path, doc_path));
    if d.variables.datasets.iter().any(|set| set.values.values().any(hidden)) {
        for v in d.variables.datasets.iter_mut().flat_map(|set| set.values.values_mut()) {
            if hidden(&*v)
                && let DataValue::Image(path) = v
            {
                *path = LinkInfo::new(path.as_str()).name().to_string();
            }
        }
    }
    for f in d.svg_scripts.iter_mut().filter(|f| script_hidden(f, doc_path)) {
        *f = LinkInfo::new(f.as_str()).name().to_string();
    }
    Cow::Owned(d)
}

// ---------- commands ----------

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = ids_param(p, "ids", "links.check")?;
    let (mut missing, mut modified) = (0, 0);
    let mut links = vec![];
    for g in groups(&st.doc, ids.as_deref()).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, LinkBase::of(st));
        missing += usize::from(status == Status::Missing);
        modified += usize::from(status == Status::Modified);
        let doc_path = LinkBase::of(st).doc;
        let mut row = json!({ "name": g.link.name(), "status": status.as_str(), "ids": ids_json(&g.ids) });
        note_path(&mut row, "path", &g.link.path, doc_path);
        note_refused(&mut row, &g.link, doc_path);
        if let Some(f) = found.filter(|f| f.path != g.link.path) {
            note_path(&mut row, "found", &f.path, doc_path);
        }
        if stale(&st.doc, g) {
            row["preview"] = json!(true);
        }
        links.push(row);
    }
    Ok(json!({ "links": links, "missing": missing, "modified": modified }))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    update_where(s, ids_param(p, "ids", "links.update")?, None)
}

/// Update Links in the active document of `s` for objects `ids` (default: the linked files that
/// changed or show their preview), or with `path`, for the placed documents of the file at `path`
/// whatever their status. The objects in symbols, graph designs, patterns and opacity masks that
/// show the same content are read again too.
fn update_where(s: &mut Session, ids: Option<Vec<NodeId>>, path: Option<&str>) -> Result<Value> {
    let st = s.doc()?;
    let (mut reads, mut missing) = (vec![], vec![]);
    let mut image_reads = ImageReads::new();
    let asked = ids.is_some() || path.is_some();
    for g in groups(&st.doc, ids.as_deref()).into_iter().filter(|g| path.is_none_or(|p| g.document.is_some() && g.link.path == p)) {
        match probe(&g, LinkBase::of(st)) {
            (Status::Missing, _) | (_, None) => missing.extend(ids_json(&g.ids)),
            // Unchanged and showing the file: nothing to read unless asked for.
            (Status::Ok, Some(_)) if !asked && !stale(&st.doc, &g) => {}
            // Unreadable as an image (or a document) counts as missing.
            (_, Some(mut f)) => match read_for(&mut f, &g, "links.update", &mut image_reads) {
                Ok(read) => reads.push((g, read)),
                Err(_) => missing.extend(ids_json(&g.ids)),
            },
        }
    }
    let updated: Vec<u64> = reads.iter().flat_map(|(g, _)| ids_json(&g.ids)).collect();
    let warnings = spot_swatch_notes(&st.doc, reads.iter().map(|(_, read)| read));
    if !reads.is_empty() {
        s.edit("Update Links", |d, _| reads.into_iter().try_for_each(|(g, read)| install_read(d, &g.ids, read, None, Some(&g))))?;
    }
    let mut out = json!({ "updated": updated, "missing": missing });
    if !warnings.is_empty() {
        out["warnings"] = json!(warnings);
    }
    Ok(out)
}

/// The warnings for the spot plates of the DCS files in `reads` whose ink a swatch of `d` already
/// names in another color, or as a process color ([`super::place::dcs::swatch_notes`]), or that an
/// earlier file of `reads` adds as a spot swatch in another color, each once.
fn spot_swatch_notes<'a>(d: &Document, reads: impl Iterator<Item = &'a Read>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    // The spot swatches the reads add, in the order they are installed.
    let mut added: Vec<(String, [f32; 4])> = vec![];
    for read in reads {
        let Read::Image((_, img)) = read else { continue };
        let spots = img.blob.spot_plates();
        let mut notes = super::place::dcs::swatch_notes(d, &spots);
        for (name, cmyk) in spots.into_iter().filter(|(name, _)| d.swatch(name).is_none()) {
            match added.iter().find(|(n, _)| *n == name) {
                Some((_, first)) if first.iter().zip(cmyk).any(|(a, b)| (a - b).abs() >= 0.005) => {
                    notes.push(format!("the document's swatch `{name}` is kept: it is a different color"));
                }
                Some(_) => {}
                None => added.push((name, cmyk)),
            }
        }
        for note in notes {
            if !out.contains(&note) {
                out.push(note);
            }
        }
    }
    out
}

/// Which links `links.relink` points at the new file (the Replace choice of Relink).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// Each link its own file (`paths`).
    Each,
    /// The objects given or selected.
    Selected,
    /// Every stored object linked to the same file as one given or selected.
    All,
    /// The objects given or selected, then every other missing link whose file name is in the new
    /// file's folder.
    MissingInFolder,
}

impl Scope {
    fn from_id(id: &str) -> Option<Self> {
        match id {
            "each" => Some(Scope::Each),
            "selected" => Some(Scope::Selected),
            "all" => Some(Scope::All),
            "missingInFolder" => Some(Scope::MissingInFolder),
            _ => None,
        }
    }
}

/// The error for `scope` and `paths` given one without the other, or `paths` beside ids, path or
/// folder.
fn each_mismatch(cmd: &str) -> EngineError {
    bad(cmd, "scope each and paths go together: paths lists entries of ids and path, in place of ids, path and folder")
}

/// `links.relink {scope: "each", paths}`: each entry's ids with its absolute file, entries naming
/// one file merged. A malformed entry, an id in two entries or an empty list is an error.
fn each_paths(p: &Value, cmd: &str) -> Result<Vec<(Vec<NodeId>, String)>> {
    let entries = p.get("paths").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| each_mismatch(cmd))?;
    let mut seen = BTreeSet::new();
    let mut out: Vec<(Vec<NodeId>, String)> = vec![];
    for (i, e) in entries.iter().enumerate() {
        let malformed = || bad(cmd, format!("paths entry {i} needs path and ids (object ids no other entry has)"));
        let path = e.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(malformed)?;
        let ids = e.get("ids").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(malformed)?;
        let mut these = vec![];
        for v in ids {
            let id = v.as_u64().map(NodeId).filter(|id| seen.insert(*id)).ok_or_else(malformed)?;
            these.push(id);
        }
        let path = absolute_path(path);
        match out.iter_mut().find(|(_, p)| *p == path) {
            Some((ids, _)) => ids.extend(these),
            None => out.push((these, path)),
        }
    }
    Ok(out)
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.relink";
    // How the new file takes the image's place this time (default: its Placement Options).
    let preserve = match str_param(p, "preserve") {
        None => None,
        Some(v) => Some(Preserve::from_id(v).ok_or_else(|| bad(C, format!("preserve `{v}` is not transforms|bounds|fileDimensions|fit|fill")))?),
    };
    let scope = match p.get("scope").filter(|v| !v.is_null()) {
        None => Scope::Selected,
        Some(v) => v.as_str().and_then(Scope::from_id).ok_or_else(|| bad(C, format!("scope {v} is not each|selected|all|missingInFolder")))?,
    };
    let st = s.doc()?;
    let folder_param = str_param(p, "folder");
    match scope {
        Scope::Each if p.get("path").is_some() || folder_param.is_some() || p.get("ids").is_some() => return Err(each_mismatch(C)),
        Scope::Selected | Scope::All | Scope::MissingInFolder if p.get("paths").is_some() => return Err(each_mismatch(C)),
        Scope::MissingInFolder if folder_param.is_some() => {
            return Err(bad(C, "scope missingInFolder takes path: the folder of that file is searched for the other missing links"));
        }
        _ => {}
    }
    let each = if scope == Scope::Each { Some(each_paths(p, C)?) } else { None };
    // With copy: the saved document's folder, which the file is copied into.
    let copy_to = match (bool_or(p, "copy", false), str_param(p, "path").is_some() || each.is_some()) {
        (false, _) => None,
        (true, false) => return Err(bad(C, "copy needs path: the file to copy into the document's folder")),
        (true, true) => {
            let doc = st.path.as_deref().map(absolute_path).ok_or_else(|| bad(C, "save the document first: copy puts the file in its folder"))?;
            // The document's path is not the client's: the error names it as the request may see it.
            let no_folder = || fileio::naming_in_error(bad(C, format!("{doc} has no folder")), &doc);
            Some(Path::new(&doc).parent().map(Path::to_path_buf).ok_or_else(no_folder)?)
        }
    };
    // An image or a placed document, with its link.
    let image = |id: &NodeId| {
        stored(&st.doc, *id).and_then(|n| match &n.kind {
            NodeKind::Image(im) => Some(im.link.as_ref()),
            NodeKind::PlacedDocument(p) => Some(Some(&p.link)),
            _ => None,
        })
    };
    let mut ids: Vec<NodeId> = match (&each, ids_param(p, "ids", C)?) {
        (Some(each), _) => each.iter().flat_map(|(ids, _)| ids.iter().copied()).collect(),
        (None, Some(ids)) => ids,
        (None, None) => {
            let selected: Vec<NodeId> = st.selection.objects.iter().copied().filter(|id| image(id).is_some()).collect();
            match folder_param {
                Some(_) if selected.is_empty() => {
                    groups(&st.doc, None).into_iter().filter(|g| probe(g, LinkBase::of(st)).0 == Status::Missing).flat_map(|g| g.ids).collect()
                }
                _ => selected,
            }
        }
    };
    if let Some(id) = ids.iter().find(|id| image(id).is_none()) {
        return Err(bad(C, format!("object {} is not an image or a linked file", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the images to relink, or pass ids"));
    }
    // All Instances: the other stored objects linked to the same files, each once.
    if scope == Scope::All {
        let paths: BTreeSet<&str> = ids.iter().filter_map(|id| image(id).flatten()).map(|l| l.path.as_str()).collect();
        let mut given: BTreeSet<NodeId> = ids.iter().copied().collect();
        let mut more = vec![];
        st.doc.visit_stored(|n, _| {
            if let Some((l, _, _)) = linked(n)
                && paths.contains(l.path.as_str())
                && n.id.0 != 0
                && given.insert(n.id)
            {
                more.push(n.id);
            }
        });
        ids.extend(more);
    }
    if copy_to.is_some() && ids.iter().any(|id| matches!(stored(&st.doc, *id).map(|n| &n.kind), Some(NodeKind::PlacedDocument(_)))) {
        return Err(bad(C, "copy takes images only: relink placed documents where their files are"));
    }
    // (ids, the file) per file to read.
    let mut files: Vec<(Vec<NodeId>, String)> = vec![];
    let mut not_found = vec![];
    match (each, str_param(p, "path"), folder_param) {
        (Some(each), _, _) => files = each,
        (None, Some(path), _) if scope == Scope::MissingInFolder => {
            let path = absolute_path(path);
            let given: BTreeSet<NodeId> = ids.iter().copied().collect();
            files.push((ids, path));
            missing_in_folder(st, &given, copy_to.is_some(), &mut files, &mut not_found);
        }
        (None, Some(path), _) => files.push((ids, absolute_path(path))),
        (None, None, Some(dir)) => {
            for id in ids {
                let Some(link) = image(&id).flatten() else { continue };
                let path = absolute_path(&Path::new(dir).join(link.name()).to_string_lossy());
                match files.iter_mut().find(|(_, p)| *p == path) {
                    Some((ids, _)) => ids.push(id),
                    None if file_stamp(&path).is_some() => files.push((vec![id], path)),
                    None => not_found.push(link.name().to_string()),
                }
            }
        }
        (None, None, None) => return Err(bad(C, "give path (a file) or folder")),
    }
    // Every file is read and checked before any is copied.
    let mut checked = vec![];
    for (ids, path) in files {
        let bytes = read_file(&path)?;
        // Images show the file; placed documents each show their artboard or page of it.
        let mut images = vec![];
        let mut documents: Vec<((u32, bool, PageBox), Vec<NodeId>)> = vec![];
        for id in ids {
            match stored(&st.doc, id).map(|n| &n.kind) {
                Some(NodeKind::PlacedDocument(pl)) => {
                    let k = (pl.page(), pl.bounding, pl.page_box);
                    match documents.iter_mut().find(|(e, _)| *e == k) {
                        Some((_, ids)) => ids.push(id),
                        None => documents.push((k, vec![id])),
                    }
                }
                _ => images.push(id),
            }
        }
        // Read before it is copied: a file that can't be shown isn't copied.
        let img = if images.is_empty() { None } else { Some(linked_image(&bytes, Some(&path)).map_err(|e| bad(C, format!("{path}: {e}")))?) };
        // A copy beside the document would point at plate files it can't find there.
        if copy_to.is_some() && img.as_ref().is_some_and(|img| !img.blob.plate_files().is_empty()) {
            return Err(bad(C, format!("{path}: copy takes a DCS file only when its plates are inside it; relink to it where its plate files are")));
        }
        let mut sources = vec![];
        for ((page, bounding, page_box), ids) in documents {
            sources.push((page, ids, super::place::document::source(&path, &bytes, Some(&path), page, bounding, page_box, C)?));
        }
        // The bytes are kept only to be copied, the file as it was checked.
        let link = link_info(&path, &bytes);
        let bytes = copy_to.is_some().then_some(bytes);
        checked.push((path, link, bytes, images, img, sources));
    }
    let mut reads = vec![];
    let (mut copied, mut kept): (Vec<Value>, Vec<Value>) = (vec![], vec![]);
    for (path, link, bytes, images, img, sources) in checked {
        let link = match (&copy_to, bytes) {
            (Some(dir), Some(bytes)) => {
                let (to, new) = copy_or_remove(&path, &bytes, dir, &copied)?;
                let link = link_info(&to, &bytes);
                let pair = json!({ "from": path, "to": to });
                if new {
                    copied.push(pair)
                } else {
                    kept.push(pair)
                }
                link
            }
            _ => link,
        };
        if let Some(img) = img {
            reads.push((images, Read::Image((link.clone(), img))));
        }
        for (page, ids, src) in sources {
            reads.push((ids, Read::Document(LinkInfo { page: Some(page), ..link.clone() }, src)));
        }
    }
    let relinked: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    let warnings = spot_swatch_notes(&s.doc()?.doc, reads.iter().map(|(_, read)| read));
    if !reads.is_empty() {
        s.edit("Relink", |d, _| reads.into_iter().try_for_each(|(ids, read)| install_read(d, &ids, read, preserve, None)))?;
    }
    not_found.sort();
    not_found.dedup();
    let mut out = json!({ "relinked": relinked, "notFound": not_found });
    if !warnings.is_empty() {
        out["warnings"] = json!(warnings);
    }
    if copy_to.is_some() {
        // The copies are in the document's folder, which the client did not give.
        for pair in copied.iter_mut().chain(kept.iter_mut()) {
            if let Some(to) = pair.get("to").and_then(Value::as_str).map(str::to_string) {
                pair["to"] = json!(fileio::shown_path(&to));
            }
        }
        out["copied"] = json!(copied);
        out["kept"] = json!(kept);
    }
    Ok(out)
}

/// `links.relink {scope: "missingInFolder"}`: the other missing links of `st` (not `given`; only
/// images when `images_only`, as copy takes), each added to `files` with the file of its name in
/// the folder of the first file there; names not there go to `not_found`.
fn missing_in_folder(
    st: &crate::DocState,
    given: &BTreeSet<NodeId>,
    images_only: bool,
    files: &mut Vec<(Vec<NodeId>, String)>,
    not_found: &mut Vec<String>,
) {
    let Some(dir) = files.first().and_then(|(_, p)| Path::new(p).parent()).map(Path::to_path_buf) else { return };
    for g in groups(&st.doc, None).into_iter().filter(|g| !(images_only && g.document.is_some())) {
        let rest: Vec<NodeId> = g.ids.iter().copied().filter(|id| !given.contains(id)).collect();
        if rest.is_empty() || probe(&g, LinkBase::of(st)).0 != Status::Missing {
            continue;
        }
        let found = absolute_path(&dir.join(g.link.name()).to_string_lossy());
        match files.iter_mut().find(|(_, p)| *p == found) {
            Some((ids, _)) => ids.extend(rest),
            None if file_stamp(&found).is_some() => files.push((rest, found)),
            None => not_found.push(g.link.name().to_string()),
        }
    }
}

/// [`copy_into_folder`]; when it fails, the copies `copied` (`{from, to}`) the same call made
/// before are removed, since nothing is relinked.
fn copy_or_remove(from: &str, bytes: &[u8], dir: &Path, copied: &[Value]) -> Result<(String, bool)> {
    copy_into_folder(from, bytes, dir).inspect_err(|_| {
        for to in copied.iter().filter_map(|pair| pair.get("to").and_then(Value::as_str)) {
            let _ = std::fs::remove_file(to);
        }
    })
}

/// `links.relink {copy}`: put the file at `from` (`bytes`, as just read) into the folder `dir`,
/// never replacing a file there ([`fileio::copy_new`]) → where it is now, and whether it was
/// copied (`false`: a file there with the same contents, of its name or a numbered one, is used).
#[cfg(not(target_arch = "wasm32"))]
fn copy_into_folder(from: &str, bytes: &[u8], dir: &Path) -> Result<(String, bool)> {
    use std::io::Write as _;
    let source = Path::new(from);
    let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Each name it takes is checked against the automation roots in force. The folder is the
    // document's, so a refusal names it and the file as the request may see them.
    let create = |to: &Path| {
        let to_text = to.to_string_lossy();
        crate::file_access::check_write(&to_text).map_err(|e| {
            let e = fileio::naming_in_error(EngineError::Other(e), &to_text);
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.to_string())
        })?;
        vectorcraft_format::write_new_with(to, |f| f.write_all(bytes))
    };
    // A file there that automation may not read isn't taken for the same file.
    let same = |to: &Path, m: &std::fs::Metadata| {
        m.len() == bytes.len() as u64 && crate::file_access::check_read(&to.to_string_lossy()).is_ok() && std::fs::read(to).is_ok_and(|b| b == bytes)
    };
    match fileio::copy_new(source, dir, create, same) {
        Ok(Some(fileio::Placed::Copied(to))) => Ok((to.to_string_lossy().into_owned(), true)),
        Ok(Some(fileio::Placed::Kept(to))) => Ok((to.to_string_lossy().into_owned(), false)),
        Ok(None) => Err(EngineError::Other(format!("every numbered name for {name} is taken in the document's folder"))),
        Err(e) => {
            let dir = dir.to_string_lossy();
            Err(fileio::naming_in_error(EngineError::Other(format!("{dir}: {e}")), &dir))
        }
    }
}

/// The web has no file system: nothing is copied.
#[cfg(target_arch = "wasm32")]
fn copy_into_folder(_: &str, _: &[u8], _: &Path) -> Result<(String, bool)> {
    Err(bad("links.relink", "copying a file isn't available on the web"))
}

// ---------- the Links panel ----------

/// Image object `id` of `d`, wherever it is stored.
fn image_of(d: &Document, id: NodeId) -> Option<&ImageObject> {
    match &stored(d, id)?.kind {
        NodeKind::Image(im) => Some(im),
        _ => None,
    }
}

/// The selected image objects of `st` that `keep` accepts.
fn selected_images(st: &crate::DocState, keep: impl Fn(&ImageObject) -> bool) -> Vec<NodeId> {
    st.selection.objects.iter().copied().filter(|id| image_of(&st.doc, *id).is_some_and(&keep)).collect()
}

/// `id`, else the first selected image.
fn image_param(st: &crate::DocState, p: &Value, cmd: &str) -> Result<NodeId> {
    let id = object_param(p, "id", cmd)?
        .or_else(|| selected_images(st, |_| true).first().copied())
        .ok_or_else(|| bad(cmd, "select an image, or pass id"))?;
    match image_of(&st.doc, id) {
        Some(_) => Ok(id),
        None if stored(&st.doc, id).is_none() => Err(crate::EngineError::NoNode(id)),
        None => Err(bad(cmd, format!("object {} is not an image", id.0))),
    }
}

/// The Placement Options of image or placed document `n`.
fn placement_mut(n: &mut Node) -> Option<&mut PlacementOptions> {
    match &mut n.kind {
        NodeKind::Image(im) => Some(&mut im.placement),
        NodeKind::PlacedDocument(p) => Some(&mut p.placement),
        _ => None,
    }
}

/// The Placement Options of image or placed document `id` of `d`, wherever it is stored.
fn placement_of(d: &Document, id: NodeId) -> Option<PlacementOptions> {
    match &stored(d, id)?.kind {
        NodeKind::Image(im) => Some(im.placement),
        NodeKind::PlacedDocument(p) => Some(p.placement),
        _ => None,
    }
}

/// `id`, else the first selected image or placed document.
fn linked_param(st: &crate::DocState, p: &Value, cmd: &str) -> Result<NodeId> {
    let first = || st.selection.objects.iter().copied().find(|id| placement_of(&st.doc, *id).is_some());
    let id = object_param(p, "id", cmd)?.or_else(first).ok_or_else(|| bad(cmd, "select an image, or pass id"))?;
    match placement_of(&st.doc, id) {
        Some(_) => Ok(id),
        None if stored(&st.doc, id).is_none() => Err(crate::EngineError::NoNode(id)),
        None => Err(bad(cmd, format!("object {} is not an image or a linked file", id.0))),
    }
}

/// Per linked image or placed document of `st`, wherever it is stored: its file's status, where it was found when away from
/// its path, and whether the image shows its preview (each file probed once).
pub(crate) fn statuses(st: &crate::DocState) -> BTreeMap<NodeId, (Status, Option<String>, bool)> {
    let mut out = BTreeMap::new();
    for g in groups(&st.doc, None).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, LinkBase::of(st));
        let found = found.map(|f| f.path).filter(|p| *p != g.link.path);
        let preview = stale(&st.doc, g);
        out.extend(g.ids.iter().map(|id| (*id, (status, found.clone(), preview))));
    }
    out
}

/// The format of the file image `im` of `d` shows: its link's extension, else its pixels' type.
fn format_label(d: &Document, im: &ImageObject) -> &'static str {
    if im.link.is_some() && d.images.get(&im.key).is_some_and(ImageBlob::is_separated) {
        return "DCS";
    }
    im.link
        .as_ref()
        .and_then(|l| fileio::format_for_name(l.name()).or_else(|| fileio::readable_format_for_name(l.name())))
        .or_else(|| d.images.get(&im.key).and_then(|b| fileio::FORMATS.iter().find(|f| f.raster && f.mime == b.mime)))
        .map_or("Image", |f| f.label)
}

/// A placed document's `links.list` fields: `format` (`VectorCraft`, or the PDF or `.ai` file's
/// format), `document: true` and its artboard's or page's size (`pageWidth`, `pageHeight`, pt)
/// instead of pixels.
fn document_row(p: &PlacedDocument) -> Value {
    let pdf = fileio::format_for_name(p.link.name()).filter(|f| matches!(f.id, "pdf" | "ai" | "ait"));
    json!({ "format": pdf.map_or("VectorCraft", |f| f.label), "document": true, "pageWidth": p.width, "pageHeight": p.height })
}

/// Sort rank of a `links.list` status: missing, modified, ok, embedded.
fn status_rank(status: &str) -> usize {
    ["missing", "modified", "ok", "embedded"].iter().position(|s| *s == status).unwrap_or(4)
}

/// The DCS images in the layers of `d` that are transparent (themselves, or a group or layer that
/// holds them) or overlap an object that is, as Show DCS Transparency Interactions marks them.
fn dcs_transparency(d: &Document) -> BTreeSet<NodeId> {
    /// Each visible DCS image's id, bounds and whether it is transparent, and the bounds of each
    /// other visible object with its own transparency; bounds are cut to `clip`, the bounds of the
    /// clipping paths of the clip groups and layers holding the object.
    fn walk(d: &Document, n: &Node, held: bool, clip: Option<Rect>, images: &mut Vec<(NodeId, Rect, bool)>, see_through: &mut Vec<Rect>) {
        if !n.visible {
            return;
        }
        let own = n.shows_own_transparency();
        // The part of the object that shows through `clip`; none when it is clipped away.
        let shown = n.geometric_bounds().and_then(|b| match clip {
            Some(c) => Some(c.intersect(b)).filter(|i| i.area() > 0.0),
            None => Some(b),
        });
        match &n.kind {
            NodeKind::Image(im) if d.images.get(&im.key).is_some_and(ImageBlob::is_separated) => {
                if let Some(b) = shown {
                    images.push((n.id, b, held || own));
                    // A transparent DCS image is an object with its own transparency for the
                    // others it overlaps.
                    if own {
                        see_through.push(b);
                    }
                }
            }
            _ if own => see_through.extend(shown),
            _ => {}
        }
        let Some(children) = n.children() else { return };
        // A clip group's or clipping layer's first child clips the others.
        let clip = match &n.kind {
            NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. } => match children.first().and_then(|c| c.geometric_bounds()) {
                Some(c) => Some(clip.map_or(c, |o| o.intersect(c))),
                None => clip,
            },
            _ => clip,
        };
        for c in children {
            walk(d, c, held || own, clip, images, see_through);
        }
    }
    let (mut images, mut see_through) = (vec![], vec![]);
    for l in &d.layers {
        walk(d, l, false, None, &mut images, &mut see_through);
    }
    images
        .into_iter()
        .filter(|(_, b, transparent)| *transparent || see_through.iter().any(|t| t.intersect(*b).area() > 0.0))
        .map(|(id, ..)| id)
        .collect()
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.list";
    let st = s.doc()?;
    let show = str_param(p, "show").unwrap_or("all");
    if !matches!(show, "all" | "missing" | "modified" | "embedded") {
        return Err(bad(C, format!("show `{show}`: all, missing, modified or embedded")));
    }
    let interactions = if switch_or(p, "dcsTransparency", false, C)? { dcs_transparency(&st.doc) } else { BTreeSet::new() };
    let statuses = statuses(st);
    // The layer tree's objects, top first as the Layers panel lists them, then the others in the
    // order they are stored (opacity-mask art, symbols, graph designs, patterns), each once.
    let (mut objects, mut nested) = (vec![], vec![]);
    let mut seen = std::collections::HashSet::new();
    st.doc.visit_stored(|n, place| {
        if matches!(n.kind, NodeKind::Image(_) | NodeKind::PlacedDocument(_)) && n.id.0 != 0 && seen.insert(n.id) {
            if place.in_layers() { objects.push((n, place)) } else { nested.push((n, place)) }
        }
    });
    objects.reverse();
    objects.extend(nested);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rows = vec![];
    for (n, place) in objects {
        let (status, found, preview) = match statuses.get(&n.id) {
            Some((s, f, p)) => (s.as_str(), f.as_deref(), *p),
            None => ("embedded", None, false),
        };
        *counts.entry(status).or_default() += 1;
        if show != "all" && show != status {
            continue;
        }
        let (link, mut row) = match &n.kind {
            NodeKind::Image(im) => {
                let mut row = json!({ "format": format_label(&st.doc, im), "pixelWidth": im.width, "pixelHeight": im.height });
                // Only an image that could be past the decode allowance (at 16 bytes a pixel, the
                // most a decoder gives) has its header read for this.
                let large = im.width as u64 * im.height as u64 > fileio::MAX_RASTER_ALLOC / 16;
                if let Some(note) = st.doc.images.get(&im.key).filter(|_| large).and_then(|b| fileio::size_note(&b.bytes)) {
                    row["note"] = json!(note);
                }
                (im.link.as_ref(), row)
            }
            NodeKind::PlacedDocument(p) => (Some(&p.link), document_row(p)),
            _ => continue,
        };
        let name = link.map_or_else(|| n.display_name(), |l| l.name().to_string());
        if let Some(o) = row.as_object_mut() {
            o.insert("id".into(), json!(n.id.0));
            o.insert("name".into(), json!(name));
            o.insert("linked".into(), json!(link.is_some()));
            o.insert("status".into(), json!(status));
        }
        let doc_path = LinkBase::of(st).doc;
        if let Some(l) = link {
            note_path(&mut row, "path", &l.path, doc_path);
            if let Some(page) = l.page {
                row["page"] = json!(page);
            }
        }
        if let Some(f) = found {
            note_path(&mut row, "found", f, doc_path);
        }
        if preview {
            row["preview"] = json!(true);
        }
        if let Some(owner) = owner_json(&st.doc, place) {
            row["owner"] = owner;
        }
        if interactions.contains(&n.id) {
            row["dcsTransparency"] = json!(true);
        }
        rows.push(row);
    }
    let text = |r: &Value, k: &str| r[k].as_str().unwrap_or_default().to_lowercase();
    match str_param(p, "sort") {
        None => {}
        Some("name") => rows.sort_by_cached_key(|r| text(r, "name")),
        Some("kind") => rows.sort_by_cached_key(|r| (text(r, "format"), text(r, "name"))),
        Some("status") => rows.sort_by_cached_key(|r| (status_rank(r["status"].as_str().unwrap_or_default()), text(r, "name"))),
        Some(x) => return Err(bad(C, format!("sort `{x}`: name, kind or status"))),
    }
    let count = |k: &str| counts.get(k).copied().unwrap_or(0);
    Ok(json!({ "links": rows, "missing": count("missing"), "modified": count("modified"), "embedded": count("embedded") }))
}

fn go_to(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let id = linked_param(st, p, "links.goTo")?;
    // An object in mask art or a definition has no place of its own on the canvas: the selection
    // is cleared, and the reply names what holds it.
    if let Some(owner) = st.doc.stored_node(id).and_then(|(_, place)| owner_json(&st.doc, place)) {
        s.select(|_, sel| sel.set([]))?;
        return Ok(json!({ "id": id.0, "onCanvas": false, "owner": owner }));
    }
    let b = st.doc.node(id).and_then(Node::geometric_bounds).unwrap_or_default();
    s.select(|_, sel| sel.set([id]))?;
    Ok(json!({ "id": id.0, "bounds": [b.x0, b.y0, b.x1, b.y1], "onCanvas": true }))
}

fn embed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.embed";
    let st = s.doc()?;
    let kind = |id: &NodeId| stored(&st.doc, *id).map(|n| &n.kind);
    let linked = |id: &NodeId| match kind(id) {
        Some(NodeKind::Image(im)) => im.link.is_some(),
        Some(NodeKind::PlacedDocument(_)) => true,
        _ => false,
    };
    let ids = ids_param(p, "ids", C)?.unwrap_or_else(|| st.selection.objects.iter().copied().filter(linked).collect());
    if let Some(id) = ids.iter().find(|id| !matches!(kind(id), Some(NodeKind::Image(_) | NodeKind::PlacedDocument(_)))) {
        return Err(bad(C, format!("object {} is not an image or a placed document", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the linked images or placed documents, or pass ids"));
    }
    // A DCS file's plates print from its file.
    let separated = |id: &NodeId| match kind(id) {
        Some(NodeKind::Image(im)) => st.doc.images.get(&im.key).is_some_and(ImageBlob::is_separated),
        _ => false,
    };
    if let Some(id) = ids.iter().find(|id| separated(id)) {
        return Err(bad(C, format!("object {}: {}", id.0, super::place::dcs::LINK_ONLY)));
    }
    // Placed documents become editable copies of their art: those whose file can be read.
    let (mut documents, mut missing) = (vec![], vec![]);
    for id in ids.iter().copied() {
        let Some(NodeKind::PlacedDocument(p)) = kind(&id) else { continue };
        let readable = st.doc.images.get(&p.key).is_some_and(|b| vectorcraft_doc::placed_document::full_bytes(p, b).is_some());
        if readable { documents.push(id) } else { missing.push(id) }
    }
    let ids: Vec<NodeId> = ids.into_iter().filter(|id| matches!(kind(id), Some(NodeKind::Image(_)))).collect();
    // Images showing their preview take their file's pixels first.
    let (mut done, mut reads) = (vec![], vec![]);
    let mut image_reads = ImageReads::new();
    for g in groups(&st.doc, Some(&ids)) {
        if !previewing(&st.doc, &g.key) {
            done.extend(g.ids);
            continue;
        }
        match probe(&g, LinkBase::of(st)) {
            (Status::Ok | Status::Modified, Some(mut f)) => match read_image(&mut f, &g.link, &mut image_reads) {
                // One showing its preview is a DCS file only its file tells.
                Ok((_, img)) if img.blob.is_separated() => {
                    return Err(bad(C, format!("object {}: {}", g.ids.first().map_or(0, |id| id.0), super::place::dcs::LINK_ONLY)));
                }
                Ok(read) => {
                    done.extend(&g.ids);
                    reads.push((g.ids, read));
                }
                Err(_) => missing.extend(g.ids),
            },
            _ => missing.extend(g.ids),
        }
    }
    if !done.is_empty() || !documents.is_empty() {
        s.edit("Embed", |d, _| {
            for (ids, read) in reads {
                install(d, &ids, read, Some(Preserve::Bounds), None)?;
            }
            for id in &done {
                if let Some(Node { kind: NodeKind::Image(im), .. }) = d.stored_node_mut(*id) {
                    im.link = None;
                }
            }
            documents.iter().try_for_each(|id| super::place::document::expand(d, *id, C))
        })?;
    }
    done.extend(documents);
    Ok(json!({ "embedded": ids_json(&done), "missing": ids_json(&missing) }))
}

/// The raster formats Unembed writes, by extension.
const UNEMBED_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp"];

/// Encoded image `bytes` re-encoded in the format of extension `ext` (JPEG without alpha).
fn convert_image(bytes: &[u8], ext: &str) -> std::result::Result<Vec<u8>, String> {
    let format = image::ImageFormat::from_extension(ext).ok_or_else(|| format!("can't write .{ext} images"))?;
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if format == image::ImageFormat::Jpeg { image::DynamicImage::ImageRgb8(img.to_rgb8()) } else { img };
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), format).map_err(|e| e.to_string())?;
    Ok(out)
}

fn unembed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.unembed";
    let st = s.doc()?;
    let id = image_param(st, p, C)?;
    let n = stored(&st.doc, id).ok_or(crate::EngineError::NoNode(id))?;
    let im = image_of(&st.doc, id).ok_or_else(|| bad(C, format!("object {} is not an image", id.0)))?;
    if im.link.is_some() {
        return Err(bad(C, format!("image {} is linked already", id.0)));
    }
    let blob = st.doc.images.get(&im.key).filter(|b| !b.is_proxy()).ok_or_else(|| bad(C, "the image's pixels aren't in the document"))?;
    // As stored (PNG, JPEG, GIF, WebP), unless the path names another format.
    let stored = fileio::FORMATS.iter().find(|f| f.raster && f.mime == blob.mime).map_or("png", |f| f.extensions.first().copied().unwrap_or("png"));
    let path = str_param(p, "path");
    let ext = match path.map(fileio::extension) {
        Some(e) if UNEMBED_EXTS.contains(&e.as_str()) => e,
        Some(e) if !e.is_empty() => return Err(bad(C, format!("can't write .{e} images: use .png, .jpg, .gif, .webp, .tif or .bmp"))),
        _ => stored.to_string(),
    };
    let same = fileio::format(&ext).is_some_and(|f| f.mime == blob.mime);
    let bytes = if same { blob.bytes.to_vec() } else { convert_image(&blob.bytes, &ext).map_err(|e| bad(C, e))? };
    let Some(path) = path else {
        let name = n.display_name();
        let stem = Path::new(&name).file_stem().map_or_else(|| name.clone(), |s| s.to_string_lossy().into_owned());
        return Ok(json!({ "name": format!("{stem}.{ext}"), "dataBase64": vectorcraft_format::base64_encode(&bytes) }));
    };
    let path = absolute_path(&if fileio::extension(path).is_empty() { format!("{path}.{ext}") } else { path.to_string() });
    // Read before writing: pixels that can't be read leave no file behind.
    let img = linked_image(&bytes, Some(&path))?;
    write_file(&path, &bytes)?;
    let read = (link_info(&path, &bytes), img);
    s.edit("Unembed", |d, _| install(d, &[id], read, Some(Preserve::Bounds), None))?;
    Ok(json!({ "id": id.0, "path": path }))
}

/// Placement Options as `{preserve, align, clip}`.
fn placement_json(o: PlacementOptions) -> Value {
    json!({ "preserve": o.preserve.id(), "align": o.align.id(), "clip": o.clip })
}

/// Link Info's fields of the linked file `l` (of object `id` of `st`) into `out`.
fn file_fields(st: &crate::DocState, id: NodeId, l: &LinkInfo, out: &mut Value) {
    // Whether the path is shown is judged for the caller asking, before the opener's roots apply.
    let shown = path_shown(&l.path, LinkBase::of(st).doc);
    // A confined caller reads within its own roots; anyone else within what the caller that
    // opened the document may read (never wider than the caller asking now).
    let roots = crate::file_access::current().or_else(|| st.opened_by.clone());
    crate::file_access::confine(roots.as_ref(), || file_fields_here(st, id, l, shown, out));
}

/// [`file_fields`]; without `shown`, the link's path (`path`, `link`) and folder (`location`)
/// and the size and date the document keeps of its file are left out.
fn file_fields_here(st: &crate::DocState, id: NodeId, l: &LinkInfo, shown: bool, out: &mut Value) {
    let base = LinkBase::of(st);
    let status = groups(&st.doc, Some(&[id])).first().map_or(Status::Missing, |g| probe(g, base).0);
    let doc_path = base.doc;
    let path = fileio::document_path(&l.path, doc_path).ok();
    let stamp = path.as_deref().and_then(file_stamp);
    out["status"] = json!(status.as_str());
    note_refused(out, l, doc_path);
    if shown {
        out["path"] = json!(l.path);
        out["location"] = json!(l.folder());
    } else if let Some(o) = out.as_object_mut() {
        for k in ["path", "link", "location"] {
            o.remove(k);
        }
    }
    out["fileName"] = json!(l.name());
    let kept = |v: Option<u64>| v.filter(|_| shown);
    for (k, v) in [
        ("page", l.page.map(u64::from)),
        ("fileSize", stamp.map(|s| s.0).or(kept(l.size))),
        ("modified", stamp.and_then(|s| s.1).or(kept(l.modified))),
        ("created", path.as_deref().and_then(file_created)),
    ] {
        if let Some(v) = v {
            out[k] = json!(v);
        }
    }
}

/// Scale (% of 100%, given points per unit at 100%) and counter-clockwise rotation of `xf`.
fn scale_rotation(xf: Affine, (px, py): (f64, f64)) -> (Value, Value) {
    let [a, b, c, d, ..] = xf.as_coeffs();
    // Counter-clockwise on the page (y points down). Adding 0 turns -0 into 0.
    (json!([a.hypot(b) / px * 100.0, c.hypot(d) / py * 100.0]), json!((-b).atan2(a).to_degrees() + 0.0))
}

/// Link Info of placed document `id`.
fn placed_info(st: &crate::DocState, id: NodeId) -> Result<Value> {
    let n = stored(&st.doc, id).ok_or(crate::EngineError::NoNode(id))?;
    let NodeKind::PlacedDocument(pl) = &n.kind else { return Err(bad("links.info", format!("object {} is not a placed document", id.0))) };
    let size = n.geometric_bounds().unwrap_or_default();
    let (scale, rotation) = scale_rotation(pl.xf, (1.0, 1.0));
    let mut out = document_row(pl);
    let fields = [
        ("id", json!(id.0)),
        ("name", json!(n.display_name())),
        ("linked", json!(true)),
        ("link", json!(pl.link.path)),
        ("width", json!(size.width())),
        ("height", json!(size.height())),
        ("scale", scale),
        ("rotation", rotation),
        ("placement", placement_json(pl.placement)),
    ];
    if let Some(o) = out.as_object_mut() {
        o.extend(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    }
    file_fields(st, id, &pl.link, &mut out);
    Ok(out)
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.info";
    let id = linked_param(s.doc()?, p, C)?;
    if image_of(&s.doc()?.doc, id).is_none() {
        return placed_info(s.doc()?, id);
    }
    let mut out = super::place::image_info(s, &json!({ "id": id.0 }))?;
    let st = s.doc()?;
    let im = image_of(&st.doc, id).ok_or_else(|| bad(C, format!("object {} is not an image", id.0)))?;
    let (px, py) = file_pt(&st.doc, &im.key);
    out["effectivePpi"] = out["ppi"].take();
    out["ppi"] = json!([72.0 / px, 72.0 / py]);
    (out["scale"], out["rotation"]) = scale_rotation(im.xf, (px, py));
    out["format"] = json!(format_label(&st.doc, im));
    out["placement"] = placement_json(im.placement);
    out["status"] = json!("embedded");
    if let Some(l) = &im.link {
        file_fields(st, id, l, &mut out);
    }
    Ok(out)
}

fn placement_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.placementOptions";
    let st = s.doc()?;
    let ids =
        ids_param(p, "ids", C)?.unwrap_or_else(|| st.selection.objects.iter().copied().filter(|id| placement_of(&st.doc, *id).is_some()).collect());
    if let Some(id) = ids.iter().find(|id| placement_of(&st.doc, **id).is_none()) {
        return Err(bad(C, format!("object {} is not an image or a linked file", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the images, or pass ids"));
    }
    let choice = |k: &str| -> Result<Option<&str>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| bad(C, format!("{k} must be a string"))),
        }
    };
    let unknown = |k: &str, v: &str| bad(C, format!("{k} `{v}` is not one of the choices"));
    let preserve = choice("preserve")?.map(|v| Preserve::from_id(v).ok_or_else(|| unknown("preserve", v))).transpose()?;
    let align = choice("align")?.map(|v| Align::from_id(v).ok_or_else(|| unknown("align", v))).transpose()?;
    let clip = match p.get("clip") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad(C, "clip must be true or false"))?),
    };
    let first = ids.first().and_then(|id| placement_of(&st.doc, *id)).unwrap_or_default();
    if preserve.is_none() && align.is_none() && clip.is_none() {
        return Ok(json!({ "placement": placement_json(first) }));
    }
    let set = |o: &mut PlacementOptions| {
        o.preserve = preserve.unwrap_or(o.preserve);
        o.align = align.unwrap_or(o.align);
        o.clip = clip.unwrap_or(o.clip);
    };
    s.edit("Placement Options", |d, _| {
        for id in &ids {
            if let Some(o) = d.stored_node_mut(*id).and_then(placement_mut) {
                set(o);
            }
        }
        Ok(())
    })?;
    let mut shown = first;
    set(&mut shown);
    Ok(json!({ "ids": ids_json(&ids), "placement": placement_json(shown) }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::tests::links::{Folder, RED, png, write};

    #[test]
    fn groups_of_one_file_share_one_decoded_image() {
        let dir = Folder::new("shared-read");
        let path = dir.file("pic.png");
        write(&path, &png(40, 20, RED));
        let (mut reads, base) = (ImageReads::new(), LinkInfo::new(path.clone()));
        let mut read = || read_image(&mut found_at(path.clone()).unwrap(), &base, &mut reads).unwrap().1;
        let (a, b) = (read(), read());
        assert!(Arc::ptr_eq(&a.blob.bytes, &b.blob.bytes), "one retained copy of the bytes");
        // A changed file is decoded again.
        write(&path, &png(40, 20, [1, 2, 3]));
        let c = read();
        assert!(!Arc::ptr_eq(&a.blob.bytes, &c.blob.bytes));
    }
}
