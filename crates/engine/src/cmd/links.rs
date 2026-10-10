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
//! Placed documents ([`PlacedDocument`], VectorCraft documents placed with Link) link to their
//! files the same way: checking, updating, relinking and finding them on open work alike, and they
//! are listed. Embed (Break Link) turns one into an editable copy of its art.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde_json::{Value, json};
use vectorcraft_doc::links::{Align, Preserve, hash_bytes};
use vectorcraft_doc::{Appearance, Document, ImageBlob, ImageObject, LinkInfo, Node, NodeId, NodeKind, PlacedDocument, PlacementOptions};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::fileio::{self, RasterImage, absolute_path, file_created, file_stamp, read_file, write_file};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "links.check",
            "Check Links",
            [],
            None,
            "{ids?: [image or placed document ids] (default: every linked image and placed document)} where each linked file is and whether it changed since it was read → {links: [{name, path, status: ok|modified|missing, ids, found?: path (found away from its path: relative to the document, or by name in its folder), preview: true (showing the low-resolution preview, not the file)}], missing, modified}",
            has_doc,
            check
        ),
        cmd!(
            "links.update",
            "Update Links",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the linked images whose file was modified or is showing its preview, and placed documents whose file was modified)} read their linked files again; each keeps its bounds. One undo step → {updated: [ids], missing: [ids]}",
            has_doc,
            update
        ),
        cmd!(
            "links.relink",
            "Relink",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected ones; with folder and nothing selected, every missing link), path | folder} link images (or placed documents, each showing the same artboard of the new file) to the file at path, or each to the file of its link's name in folder; embedded images become linked; each keeps its bounds. One undo step → {relinked: [ids], notFound: [file names]}",
            has_doc,
            relink
        ),
        cmd!(
            query "links.list",
            "Links",
            [],
            None,
            "{show?: all (default)|missing|modified|embedded, sort?: name|kind (file format)|status (missing, modified, ok, embedded); default: stacking order, top first} every image object and placed document in the layers, as the Links panel lists them → {links: [{id, name, linked, status: ok|modified|missing|embedded, format, pixelWidth, pixelHeight, path?, found?: path (found away from its path), page?, preview?: true (showing the saved preview), document?: true (a placed document: pageWidth and pageHeight in pt instead of pixels)}], missing, modified, embedded}",
            has_doc,
            list
        ),
        cmd!(
            "links.goTo",
            "Go To Link",
            [],
            None,
            "{id?} select image or placed document `id` (default: the first selected one); the Links panel scrolls it into view → {id, bounds: [x0, y0, x1, y1]}",
            has_doc,
            go_to
        ),
        cmd!(
            "links.embed",
            "Embed Image",
            [],
            None,
            "{ids?: [image or placed document ids] (default: the selected linked ones)} break the links (the Links panel's Break Link): images keep the linked files' pixels in the document; a placed document becomes an editable copy of its art, as placing its file without link gives (its symbols, patterns, swatches and images joining the document). An image whose file can't be found and that shows the saved preview, or a placed document whose file can't be read or changed, stays linked: relink or update it first. One undo step → {embedded: [ids], missing: [ids]}",
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
            "{id?} an image's or placed document's Link Info (default: the one selected) → image.info's fields (id, name, linked, link, colorMode, pixelWidth, pixelHeight, width, height) and status: ok|modified|missing|embedded, format, ppi: [x, y] (the file's), effectivePpi: [x, y] (at its placed size), scale: [x%, y%] (of its 100% size), rotation (degrees, counter-clockwise), placement: {preserve, align, clip}; linked images also fileName, location (its folder), page?, fileSize? (bytes), modified?, created? (ms since the Unix epoch); a placed document has document: true, pageWidth and pageHeight (pt) instead of pixels, ppi and colorMode, and its scale is of its artboard's 100% size",
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
/// placed documents that show one artboard of it.
struct Group {
    link: LinkInfo,
    key: String,
    /// The objects in the layers (images in symbols and patterns share the content, not an id).
    ids: Vec<NodeId>,
    /// Placed documents: whether the art's bounds (not the artboard) are shown.
    document: Option<bool>,
}

/// The linked object `n` as (link, content key, placed document's bounding).
fn linked(n: &Node) -> Option<(&LinkInfo, &str, Option<bool>)> {
    match &n.kind {
        NodeKind::Image(im) => Some((im.link.as_ref()?, &im.key, None)),
        NodeKind::PlacedDocument(p) => Some((&p.link, &p.key, Some(p.bounding))),
        _ => None,
    }
}

/// The linked images and placed documents of `d` by file and content: those in the layers (only `ids` when
/// given), then, without `ids`, the ones only symbols and patterns show.
fn groups(d: &Document, ids: Option<&[NodeId]>) -> Vec<Group> {
    let mut out: Vec<Group> = vec![];
    let mut index: BTreeMap<(String, String, bool), usize> = BTreeMap::new();
    let mut add = |link: &LinkInfo, key: &str, document: Option<bool>, id: Option<NodeId>| {
        let i = *index.entry((link.path.clone(), key.to_string(), document.is_some())).or_insert_with(|| {
            out.push(Group { link: link.clone(), key: key.to_string(), ids: vec![], document });
            out.len() - 1
        });
        if let (Some(id), Some(g)) = (id, out.get_mut(i)) {
            g.ids.push(id);
        }
    };
    d.walk(|n| {
        if let Some((l, key, document)) = linked(n)
            && ids.is_none_or(|ids| ids.contains(&n.id))
        {
            add(l, key, document, Some(n.id));
        }
    });
    if ids.is_none() {
        d.visit_images(|_, im| {
            if let Some(l) = &im.link {
                add(l, &im.key, None, None);
            }
        });
        d.visit_placed(|_, p| add(&p.link, &p.key, Some(p.bounding), None));
    }
    out
}

/// A linked file where it was found.
struct Found {
    path: String,
    size: u64,
    modified: Option<u64>,
    /// Read when telling whether it changed needed them.
    bytes: Option<Vec<u8>>,
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
        LinkInfo { path: self.path.clone(), size: Some(self.size), modified: self.modified, hash: Some(hash_bytes(bytes)), ..base.clone() }
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
    Some(Found { path, size, modified, bytes: None })
}

/// Where the file of `link` is: at its path, else at its relative path or by name in the
/// document's folder `dir`.
fn locate(link: &LinkInfo, dir: Option<&Path>) -> Option<Found> {
    let mut paths = vec![link.path.clone()];
    if let Some(dir) = dir {
        let near = link.relative.iter().map(|r| dir.join(r)).chain([dir.join(link.name())]);
        paths.extend(near.map(|p| p.to_string_lossy().into_owned()));
    }
    paths.into_iter().find_map(found_at)
}

/// Where the file of group `g` is and whether it still has the content shown. Its bytes are read
/// only when its size or modification time changed.
fn probe(g: &Group, dir: Option<&Path>) -> (Status, Option<Found>) {
    let Some(mut f) = locate(&g.link, dir) else { return (Status::Missing, None) };
    let l = &g.link;
    if l.size == Some(f.size) && l.modified.is_some() && l.modified == f.modified {
        return (Status::Ok, Some(f));
    }
    let Ok(bytes) = f.read() else { return (Status::Missing, None) };
    let same = match &l.hash {
        Some(h) => *h == hash_bytes(&bytes),
        // Linked before links kept a hash: the image the file decodes to.
        None => g.document.is_none() && fileio::raster_image(&bytes).is_ok_and(|img| img.key == g.key),
    };
    f.bytes = Some(bytes);
    (if same { Status::Ok } else { Status::Modified }, Some(f))
}

/// The folder of the document saved at `doc_path`.
fn folder(doc_path: Option<&str>) -> Option<&Path> {
    doc_path.and_then(|p| Path::new(p).parent())
}

/// Is image `key` of `d` showing its preview (its file not read)?
fn previewing(d: &Document, key: &str) -> bool {
    d.images.get(key).is_none_or(ImageBlob::is_proxy)
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

/// Every file the linked images of `d` (saved at `doc_path`) show, in the layers, symbols and
/// patterns, once each.
pub(crate) fn linked_files(d: &Document, doc_path: Option<&str>) -> Vec<LinkedFile> {
    let mut seen = std::collections::BTreeSet::new();
    groups(d, None)
        .into_iter()
        .filter(|g| seen.insert(g.link.path.clone()))
        .map(|g| {
            let own = |b: &&ImageBlob| !b.is_proxy() && fileio::format_for_name(g.link.name()).is_some_and(|f| f.mime == b.mime);
            let found = locate(&g.link, folder(doc_path));
            let found_path = found.as_ref().map(|f| f.path.clone());
            let bytes = found.and_then(|mut f| f.read().ok()).or_else(|| d.images.get(&g.key).filter(own).map(|b| b.bytes.to_vec()));
            LinkedFile { path: g.link.path.clone(), name: g.link.name().to_string(), found_path, bytes }
        })
        .collect()
}

// ---------- reading files into the document ----------

/// Keep `full` (a file's pixels) as image `key` of `d`, unless pixels are there already; a
/// preview there is kept, and a linked image gets one.
pub(crate) fn store_image(d: &mut Document, key: &str, full: ImageBlob, linked: bool) {
    let blob = match d.images.get(key) {
        Some(b) if !b.is_proxy() => b.clone(),
        old => ImageBlob { proxy: old.and_then(|b| b.proxy.clone()), ..full },
    };
    d.images.insert(key.to_string(), if linked { blob.with_proxy() } else { blob });
}

/// The file read for a link: the link to it and its image.
type FileRead = (LinkInfo, RasterImage);

/// The file at `f` read for a link like `base` (its image decoded).
fn read_image(f: &mut Found, base: &LinkInfo) -> Result<FileRead> {
    let bytes = f.read()?;
    Ok((f.link(&bytes, base), fileio::raster_image(&bytes)?))
}

/// A linked file read for the objects that show it.
enum Read {
    Image(FileRead),
    /// A VectorCraft document read for placed documents: the link to it and what they show.
    Document(LinkInfo, super::place::document::Source),
}

/// The file at `f` read for group `g`: its image, or the artboard `g` shows.
fn read_for(f: &mut Found, g: &Group, cmd: &str) -> Result<Read> {
    let Some(bounding) = g.document else { return read_image(f, &g.link).map(Read::Image) };
    let bytes = f.read()?;
    let page = g.link.page.unwrap_or(1);
    let src = super::place::document::source(&f.path, &bytes, Some(&f.path), page, bounding, cmd)?;
    Ok(Read::Document(f.link(&bytes, &g.link), src))
}

/// Objects `ids` show what was read (see [`install`], [`install_document`]).
fn install_read(d: &mut Document, ids: &[NodeId], read: Read, keep_bounds: bool) -> Result<()> {
    match read {
        Read::Image(r) => install(d, ids, r, keep_bounds),
        Read::Document(link, src) => install_document(d, ids, &link, &src, keep_bounds),
    }
}

/// Placed documents `ids` show `src`, read from the file `link` points at, each placed as its
/// Placement Options say (`keep_bounds`: in its bounds, whatever they say).
fn install_document(d: &mut Document, ids: &[NodeId], link: &LinkInfo, src: &super::place::document::Source, keep_bounds: bool) -> Result<()> {
    let mut clips = vec![];
    let (w, h) = (src.frame.width(), src.frame.height());
    for id in ids {
        let Some(NodeKind::PlacedDocument(p)) = d.node(*id).map(|n| &n.kind) else { continue };
        let o = if keep_bounds { PlacementOptions { preserve: Preserve::Bounds, ..p.placement } } else { p.placement };
        let (xf, clip) = placed_xf(p.xf, (p.width, p.height), (1.0, 1.0), (w, h), (1.0, 1.0), o);
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

/// Clip image `id` of `d` to `room` in `frame`: the clipping path of the clip group holding just
/// it changes, else the image goes into a new clip group.
fn clip_image(d: &mut Document, id: NodeId, (frame, room): Clip) -> Result<()> {
    let path = shapes::rectangle(Rect::new(0.0, 0.0, room.0, room.1)).transformed(frame);
    let clip_path = d.parent_of(id).and_then(|p| match &d.node(p)?.kind {
        NodeKind::Group { children, clip: true } => match &children[..] {
            [c, im] if im.id == id && matches!(c.kind, NodeKind::Path { clipping: true, .. }) => Some(c.id),
            _ => None,
        },
        _ => None,
    });
    if let Some(c) = clip_path {
        if let Some(Node { kind: NodeKind::Path { path: p, .. }, .. }) = d.node_mut(c) {
            *p = path;
        }
        return Ok(());
    }
    let (par, index, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    let image = d.remove(id)?;
    let none = vectorcraft_color::Paint::None;
    let mut clip = Node::path(d.alloc_id(), path, Appearance::basic(none.clone(), none, 0.0));
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let group = Node::new(d.alloc_id(), NodeKind::Group { children: vec![std::sync::Arc::new(clip), image], clip: true });
    d.insert(par, index, group)?;
    Ok(())
}

/// Image objects `ids` show the file read for `link` (`img`), each placed as its Placement
/// Options say (`keep_bounds`: in its bounds, whatever they say).
fn install(d: &mut Document, ids: &[NodeId], (link, img): FileRead, keep_bounds: bool) -> Result<()> {
    let new_pt = super::place::pt_per_px(img.ppi);
    let mut clips = vec![];
    for id in ids {
        let Some(NodeKind::Image(im)) = d.node(*id).map(|n| &n.kind) else { continue };
        let o = if keep_bounds { PlacementOptions { preserve: Preserve::Bounds, ..im.placement } } else { im.placement };
        let size = |w: u32, h: u32| (w as f64, h as f64);
        let (xf, clip) = placed_xf(im.xf, size(im.width, im.height), file_pt(d, &im.key), size(img.width, img.height), new_pt, o);
        if let Some(n) = d.node_mut(*id)
            && let NodeKind::Image(im) = &mut n.kind
        {
            (im.key, im.width, im.height, im.xf, im.link) = (img.key.clone(), img.width, img.height, xf, Some(link.clone()));
        }
        clips.extend(clip.map(|c| (*id, c)));
    }
    store_image(d, &img.key, img.blob, true);
    clips.into_iter().try_for_each(|(id, clip)| clip_image(d, id, clip))
}

/// Groups as `{name, path, ids}` rows (those with objects in the layers).
fn rows<'a>(gs: impl Iterator<Item = &'a Group>) -> Vec<Value> {
    gs.filter(|g| !g.ids.is_empty()).map(|g| json!({ "name": g.link.name(), "path": g.link.path, "ids": ids_json(&g.ids) })).collect()
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

/// The linked files of document `d`, just read from `doc_path`: those found unchanged are read
/// again (their images had the preview), modified ones are read again when `update`, links found
/// away from their path take the new path. Nothing here is an undo step.
pub fn resolve(d: &mut Document, doc_path: Option<&str>, update: bool) -> Resolved {
    let mut out = Resolved::default();
    for g in groups(d, None) {
        let row = rows(std::iter::once(&g));
        match probe(&g, folder(doc_path)) {
            (Status::Missing, _) => out.missing.extend(row),
            (Status::Modified, Some(mut f)) if update => {
                match read_for(&mut f, &g, "document.open").and_then(|read| install_read(d, &g.ids, read, false)) {
                    Ok(()) => out.updated.extend(row),
                    Err(_) => out.missing.extend(row),
                }
            }
            (Status::Modified, _) => out.modified.extend(row),
            // A placed document keeps its file's bytes: only lost ones are read again.
            (Status::Ok, Some(mut f)) if g.document.is_some() => {
                let read = if stale(d, &g) { Some(read_for(&mut f, &g, "document.open")) } else { None };
                match read {
                    Some(Ok(read)) => {
                        if install_read(d, &g.ids, read, true).is_err() {
                            out.missing.extend(row);
                        }
                    }
                    Some(Err(_)) => out.missing.extend(row),
                    None => {
                        let link = match f.bytes.take() {
                            Some(b) => f.link(&b, &g.link),
                            None => LinkInfo { path: f.path.clone(), ..g.link.clone() },
                        };
                        if link != g.link {
                            set_links(d, &g.ids, &link);
                        }
                    }
                }
            }
            (Status::Ok, Some(mut f)) => {
                let preview = previewing(d, &g.key);
                // Read when the images show the preview, or when the file's details changed.
                let bytes = if preview || f.bytes.is_some() { f.read().ok() } else { None };
                if preview {
                    let Some(img) = bytes.as_deref().and_then(|b| fileio::raster_image(b).ok()) else {
                        out.missing.extend(row);
                        continue;
                    };
                    store_image(d, &g.key, img.blob, true);
                } else if let Some(b) = d.images.get(&g.key).filter(|b| b.proxy.is_none()) {
                    // Linked before links kept a preview.
                    let b = b.clone().with_proxy();
                    d.images.insert(g.key.clone(), b);
                }
                // Found away from its path, or read again: the link follows.
                let link = match &bytes {
                    Some(b) => f.link(b, &g.link),
                    None => LinkInfo { path: f.path.clone(), ..g.link.clone() },
                };
                if link != g.link {
                    set_links(d, &g.ids, &link);
                }
            }
            (Status::Ok, None) => {}
        }
    }
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

/// Linked image objects (or placed documents) `ids` link to `link`.
fn set_links(d: &mut Document, ids: &[NodeId], link: &LinkInfo) {
    for id in ids {
        if let Some(l) = d.node_mut(*id).and_then(link_mut) {
            *l = link.clone();
        }
    }
}

/// Placed documents of the document saved at `path` (document `saved` of `s`) in the other open
/// documents: read again from the new file, each as one Update Links step of its document.
pub(crate) fn refresh_placed(s: &mut Session, saved: u64, path: &str) {
    let path = absolute_path(path);
    let targets: Vec<(usize, Vec<u64>)> = s
        .documents()
        .iter()
        .enumerate()
        .filter(|(_, st)| st.uid != saved)
        .filter_map(|(i, st)| {
            let mut ids = vec![];
            st.doc.walk(|n| {
                if let NodeKind::PlacedDocument(p) = &n.kind
                    && p.link.path == path
                {
                    ids.push(n.id.0);
                }
            });
            (!ids.is_empty()).then_some((i, ids))
        })
        .collect();
    if targets.is_empty() {
        return;
    }
    // Each document is updated as the active one, without switching tools.
    let back = s.active;
    for (i, ids) in targets {
        s.active = Some(i);
        // A file that can't be read leaves what its objects show: links.check reports it.
        let _ = update(s, &json!({ "ids": ids }));
    }
    s.active = back;
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

/// `d` as saved to `dest`: the links' paths relative to its folder written in; `None` when they
/// are already.
pub fn with_relative_paths(d: &Document, dest: &str) -> Option<Document> {
    let dest = absolute_path(dest);
    let dir = Path::new(&dest).parent()?;
    let mut changes = vec![];
    d.walk(|n| {
        if let Some((l, _, _)) = linked(n) {
            let r = relative_path(Path::new(&l.path), dir);
            if r != l.relative {
                changes.push((n.id, r));
            }
        }
    });
    if changes.is_empty() {
        return None;
    }
    let mut d = d.clone();
    for (id, r) in changes {
        if let Some(l) = d.node_mut(id).and_then(link_mut) {
            l.relative = r;
        }
    }
    Some(d)
}

// ---------- commands ----------

fn check(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = ids_param(p, "ids");
    let (mut missing, mut modified) = (0, 0);
    let mut links = vec![];
    for g in groups(&st.doc, ids.as_deref()).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, folder(st.path.as_deref()));
        missing += usize::from(status == Status::Missing);
        modified += usize::from(status == Status::Modified);
        let mut row = json!({ "name": g.link.name(), "path": g.link.path, "status": status.as_str(), "ids": ids_json(&g.ids) });
        if let Some(f) = found.filter(|f| f.path != g.link.path) {
            row["found"] = json!(f.path);
        }
        if stale(&st.doc, g) {
            row["preview"] = json!(true);
        }
        links.push(row);
    }
    Ok(json!({ "links": links, "missing": missing, "modified": modified }))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let ids = ids_param(p, "ids");
    let (mut reads, mut missing) = (vec![], vec![]);
    for g in groups(&st.doc, ids.as_deref()).into_iter().filter(|g| !g.ids.is_empty()) {
        match probe(&g, folder(st.path.as_deref())) {
            (Status::Missing, _) | (_, None) => missing.extend(ids_json(&g.ids)),
            // Unchanged and showing the file: nothing to read unless asked for by id.
            (Status::Ok, Some(_)) if ids.is_none() && !stale(&st.doc, &g) => {}
            // Unreadable as an image (or a document) counts as missing.
            (_, Some(mut f)) => match read_for(&mut f, &g, "links.update") {
                Ok(read) => reads.push((g.ids, read)),
                Err(_) => missing.extend(ids_json(&g.ids)),
            },
        }
    }
    let updated: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    if !reads.is_empty() {
        s.edit("Update Links", |d, _| reads.into_iter().try_for_each(|(ids, read)| install_read(d, &ids, read, false)))?;
    }
    Ok(json!({ "updated": updated, "missing": missing }))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.relink";
    let st = s.doc()?;
    let folder_param = str_param(p, "folder");
    // An image or a placed document, with its link.
    let image = |id: &NodeId| {
        st.doc.node(*id).and_then(|n| match &n.kind {
            NodeKind::Image(im) => Some(im.link.as_ref()),
            NodeKind::PlacedDocument(p) => Some(Some(&p.link)),
            _ => None,
        })
    };
    let ids: Vec<NodeId> = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => {
            let selected: Vec<NodeId> = st.selection.objects.iter().copied().filter(|id| image(id).is_some()).collect();
            match folder_param {
                Some(_) if selected.is_empty() => groups(&st.doc, None)
                    .into_iter()
                    .filter(|g| probe(g, folder(st.path.as_deref())).0 == Status::Missing)
                    .flat_map(|g| g.ids)
                    .collect(),
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
    // (ids, the file) per file to read.
    let mut files: Vec<(Vec<NodeId>, String)> = vec![];
    let mut not_found = vec![];
    match (str_param(p, "path"), folder_param) {
        (Some(path), _) => files.push((ids, absolute_path(path))),
        (None, Some(dir)) => {
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
        (None, None) => return Err(bad(C, "give path (a file) or folder")),
    }
    let mut reads = vec![];
    for (ids, path) in files {
        let bytes = read_file(&path)?;
        let link = link_info(&path, &bytes);
        // Images show the file; placed documents each show their artboard of it.
        let mut images = vec![];
        let mut documents: Vec<((u32, bool), Vec<NodeId>)> = vec![];
        for id in ids {
            match st.doc.node(id).map(|n| &n.kind) {
                Some(NodeKind::PlacedDocument(pl)) => {
                    let k = (pl.page(), pl.bounding);
                    match documents.iter_mut().find(|(e, _)| *e == k) {
                        Some((_, ids)) => ids.push(id),
                        None => documents.push((k, vec![id])),
                    }
                }
                _ => images.push(id),
            }
        }
        if !images.is_empty() {
            let img = fileio::raster_image(&bytes).map_err(|e| bad(C, format!("{path}: {e}")))?;
            reads.push((images, Read::Image((link.clone(), img))));
        }
        for ((page, bounding), ids) in documents {
            let src = super::place::document::source(&path, &bytes, Some(&path), page, bounding, C)?;
            reads.push((ids, Read::Document(LinkInfo { page: Some(page), ..link.clone() }, src)));
        }
    }
    let relinked: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    if !reads.is_empty() {
        s.edit("Relink", |d, _| reads.into_iter().try_for_each(|(ids, read)| install_read(d, &ids, read, false)))?;
    }
    not_found.sort();
    not_found.dedup();
    Ok(json!({ "relinked": relinked, "notFound": not_found }))
}

// ---------- the Links panel ----------

/// Image object `id` of `d`.
fn image_of(d: &Document, id: NodeId) -> Option<&ImageObject> {
    match &d.node(id)?.kind {
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
    let id = id_param(p, "id").or_else(|| selected_images(st, |_| true).first().copied()).ok_or_else(|| bad(cmd, "select an image, or pass id"))?;
    match image_of(&st.doc, id) {
        Some(_) => Ok(id),
        None if st.doc.node(id).is_none() => Err(crate::EngineError::NoNode(id)),
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

/// The Placement Options of image or placed document `id` of `d`.
fn placement_of(d: &Document, id: NodeId) -> Option<PlacementOptions> {
    match &d.node(id)?.kind {
        NodeKind::Image(im) => Some(im.placement),
        NodeKind::PlacedDocument(p) => Some(p.placement),
        _ => None,
    }
}

/// `id`, else the first selected image or placed document.
fn linked_param(st: &crate::DocState, p: &Value, cmd: &str) -> Result<NodeId> {
    let first = || st.selection.objects.iter().copied().find(|id| placement_of(&st.doc, *id).is_some());
    let id = id_param(p, "id").or_else(first).ok_or_else(|| bad(cmd, "select an image, or pass id"))?;
    match placement_of(&st.doc, id) {
        Some(_) => Ok(id),
        None if st.doc.node(id).is_none() => Err(crate::EngineError::NoNode(id)),
        None => Err(bad(cmd, format!("object {} is not an image or a linked file", id.0))),
    }
}

/// Per linked image in the layers of `st`: its file's status, where it was found when away from
/// its path, and whether the image shows its preview (each file probed once).
fn statuses(st: &crate::DocState) -> BTreeMap<NodeId, (Status, Option<String>, bool)> {
    let mut out = BTreeMap::new();
    for g in groups(&st.doc, None).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, folder(st.path.as_deref()));
        let found = found.map(|f| f.path).filter(|p| *p != g.link.path);
        let preview = stale(&st.doc, g);
        out.extend(g.ids.iter().map(|id| (*id, (status, found.clone(), preview))));
    }
    out
}

/// The format of the file image `im` of `d` shows: its link's extension, else its pixels' type.
fn format_label(d: &Document, im: &ImageObject) -> &'static str {
    im.link
        .as_ref()
        .and_then(|l| fileio::format_for_name(l.name()))
        .or_else(|| d.images.get(&im.key).and_then(|b| fileio::FORMATS.iter().find(|f| f.raster && f.mime == b.mime)))
        .map_or("Image", |f| f.label)
}

/// A placed document's `links.list` fields: `format`, `document: true` and its artboard's size
/// (`pageWidth`, `pageHeight`, pt) instead of pixels.
fn document_row(p: &PlacedDocument) -> Value {
    json!({ "format": "VectorCraft", "document": true, "pageWidth": p.width, "pageHeight": p.height })
}

/// Sort rank of a `links.list` status: missing, modified, ok, embedded.
fn status_rank(status: &str) -> usize {
    ["missing", "modified", "ok", "embedded"].iter().position(|s| *s == status).unwrap_or(4)
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.list";
    let st = s.doc()?;
    let show = str_param(p, "show").unwrap_or("all");
    if !matches!(show, "all" | "missing" | "modified" | "embedded") {
        return Err(bad(C, format!("show `{show}`: all, missing, modified or embedded")));
    }
    let statuses = statuses(st);
    let mut objects = vec![];
    st.doc.walk(|n| {
        if matches!(n.kind, NodeKind::Image(_) | NodeKind::PlacedDocument(_)) {
            objects.push(n);
        }
    });
    // Top first, as the Layers panel lists them.
    objects.reverse();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rows = vec![];
    for n in objects {
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
                (im.link.as_ref(), json!({ "format": format_label(&st.doc, im), "pixelWidth": im.width, "pixelHeight": im.height }))
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
        if let Some(l) = link {
            row["path"] = json!(l.path);
            if let Some(page) = l.page {
                row["page"] = json!(page);
            }
        }
        if let Some(f) = found {
            row["found"] = json!(f);
        }
        if preview {
            row["preview"] = json!(true);
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
    let b = st.doc.node(id).and_then(Node::geometric_bounds).unwrap_or_default();
    s.select(|_, sel| sel.set([id]))?;
    Ok(json!({ "id": id.0, "bounds": [b.x0, b.y0, b.x1, b.y1] }))
}

fn embed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.embed";
    let st = s.doc()?;
    let kind = |id: &NodeId| st.doc.node(*id).map(|n| &n.kind);
    let linked = |id: &NodeId| match kind(id) {
        Some(NodeKind::Image(im)) => im.link.is_some(),
        Some(NodeKind::PlacedDocument(_)) => true,
        _ => false,
    };
    let ids = ids_param(p, "ids").unwrap_or_else(|| st.selection.objects.iter().copied().filter(linked).collect());
    if let Some(id) = ids.iter().find(|id| !matches!(kind(id), Some(NodeKind::Image(_) | NodeKind::PlacedDocument(_)))) {
        return Err(bad(C, format!("object {} is not an image or a placed document", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the linked images or placed documents, or pass ids"));
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
    for g in groups(&st.doc, Some(&ids)) {
        if !previewing(&st.doc, &g.key) {
            done.extend(g.ids);
            continue;
        }
        match probe(&g, folder(st.path.as_deref())) {
            (Status::Ok | Status::Modified, Some(mut f)) => match read_image(&mut f, &g.link) {
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
                install(d, &ids, read, true)?;
            }
            for id in &done {
                if let Some(Node { kind: NodeKind::Image(im), .. }) = d.node_mut(*id) {
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
    let n = st.doc.node(id).ok_or(crate::EngineError::NoNode(id))?;
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
    write_file(&path, &bytes)?;
    let read = (link_info(&path, &bytes), fileio::raster_image(&bytes)?);
    s.edit("Unembed", |d, _| install(d, &[id], read, true))?;
    Ok(json!({ "id": id.0, "path": path }))
}

/// Placement Options as `{preserve, align, clip}`.
fn placement_json(o: PlacementOptions) -> Value {
    json!({ "preserve": o.preserve.id(), "align": o.align.id(), "clip": o.clip })
}

/// Link Info's fields of the linked file `l` (of object `id` of `st`) into `out`.
fn file_fields(st: &crate::DocState, id: NodeId, l: &LinkInfo, out: &mut Value) {
    let status = groups(&st.doc, Some(&[id])).first().map_or(Status::Missing, |g| probe(g, folder(st.path.as_deref())).0);
    let stamp = file_stamp(&l.path);
    let folder_len = l.path.len().saturating_sub(l.name().len());
    out["status"] = json!(status.as_str());
    out["path"] = json!(l.path);
    out["fileName"] = json!(l.name());
    out["location"] = json!(l.path.get(..folder_len).unwrap_or_default().trim_end_matches(['/', '\\']));
    for (k, v) in [
        ("page", l.page.map(u64::from)),
        ("fileSize", stamp.map(|s| s.0).or(l.size)),
        ("modified", stamp.and_then(|s| s.1).or(l.modified)),
        ("created", file_created(&l.path)),
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
    let n = st.doc.node(id).ok_or(crate::EngineError::NoNode(id))?;
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
    let ids = ids_param(p, "ids").unwrap_or_else(|| st.selection.objects.iter().copied().filter(|id| placement_of(&st.doc, *id).is_some()).collect());
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
            if let Some(o) = d.node_mut(*id).and_then(placement_mut) {
                set(o);
            }
        }
        Ok(())
    })?;
    let mut shown = first;
    set(&mut shown);
    Ok(json!({ "ids": ids_json(&ids), "placement": placement_json(shown) }))
}
