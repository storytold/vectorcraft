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

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde_json::{Value, json};
use vectorcraft_doc::links::{Align, Preserve, hash_bytes};
use vectorcraft_doc::{Appearance, Document, ImageBlob, ImageObject, LinkInfo, Node, NodeId, NodeKind, PlacementOptions};
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
            "{ids?: [image ids] (default: every linked image)} where each linked file is and whether it changed since it was read → {links: [{name, path, status: ok|modified|missing, ids, found?: path (found away from its path: relative to the document, or by name in its folder), preview: true (showing the low-resolution preview, not the file)}], missing, modified}",
            has_doc,
            check
        ),
        cmd!(
            "links.update",
            "Update Links",
            [],
            None,
            "{ids?: [image ids] (default: the linked images whose file was modified or is showing its preview)} read their linked files again; each image keeps its bounds. One undo step → {updated: [ids], missing: [ids]}",
            has_doc,
            update
        ),
        cmd!(
            "links.relink",
            "Relink",
            [],
            None,
            "{ids?: [image ids] (default: the selected images; with folder and nothing selected, every missing link), path | folder, allInstances?: bool (default true: with path, every other image linked to the same file as one of them is relinked too), sameFolder?: bool (default true: with path, other images whose files are missing are relinked to files of their names in path's folder)} link images to the file at path, or each to the file of its link's name in folder; embedded images become linked; each keeps its bounds. One undo step → {relinked: [ids], notFound: [file names], alsoRelinked: [ids found in the same folder]}",
            has_doc,
            relink
        ),
        cmd!(
            query "links.list",
            "Links",
            [],
            None,
            "{show?: all (default)|missing|modified|embedded, sort?: name|kind (file format)|status (missing, modified, ok, embedded); default: stacking order, top first} every image object in the layers, as the Links panel lists them → {links: [{id, name, linked, status: ok|modified|missing|embedded, format, pixelWidth, pixelHeight, path?, found?: path (found away from its path), page?, preview?: true (showing the saved preview)}], missing, modified, embedded}",
            has_doc,
            list
        ),
        cmd!(
            "links.goTo",
            "Go To Link",
            [],
            None,
            "{id?} select image `id` (default: the first selected image); the Links panel scrolls it into view → {id, bounds: [x0, y0, x1, y1]}",
            has_doc,
            go_to
        ),
        cmd!(
            "links.embed",
            "Embed Image",
            [],
            None,
            "{ids?: [image ids] (default: the selected linked images)} keep the linked files' pixels in the document and drop the links (an image whose file can't be found and that shows the saved preview stays linked: relink it first). One undo step → {embedded: [ids], missing: [ids]}",
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
            "{id?} an image's Link Info (default: the one selected) → image.info's fields (id, name, linked, link, colorMode, pixelWidth, pixelHeight, width, height) and status: ok|modified|missing|embedded, format, ppi: [x, y] (the file's), effectivePpi: [x, y] (at its placed size), scale: [x%, y%] (of its 100% size), rotation (degrees, counter-clockwise), placement: {preserve, align, clip}; linked images also fileName, location (its folder), page?, fileSize? (bytes), modified?, created? (ms since the Unix epoch)",
            has_doc,
            info
        ),
        cmd!(
            "links.placementOptions",
            "Placement Options",
            [],
            None,
            "{ids?: [image ids] (default: the selected images), preserve?: transforms|bounds (default)|fileDimensions|fit|fill, align?: topLeft|top|topRight|left|center (default)|right|bottomLeft|bottom|bottomRight (where the new art sits; not for bounds), clip?: false (clip it to the old bounds where it is larger)} how a file read again (links.relink, links.update) takes each image's place: transforms keeps its scale (relative to each file's 100% size), rotation and position; bounds stretches it into the old bounds; fileDimensions puts it at 100%, unrotated; fit and fill scale it proportionally to fit inside or cover the old bounds. Without options → {placement} of the first image; else one undo step → {ids, placement}",
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

/// The image objects that show one linked file as one content (same path, same image).
struct Group {
    link: LinkInfo,
    key: String,
    /// The objects in the layers (images in symbols and patterns share the content, not an id).
    ids: Vec<NodeId>,
}

/// The linked images of `d` by file and content: those in the layers (only `ids` when given), then,
/// without `ids`, the ones only symbols and patterns show.
fn groups(d: &Document, ids: Option<&[NodeId]>) -> Vec<Group> {
    let mut out: Vec<Group> = vec![];
    let mut index: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut add = |link: &LinkInfo, key: &str, id: Option<NodeId>| {
        let i = *index.entry((link.path.clone(), key.to_string())).or_insert_with(|| {
            out.push(Group { link: link.clone(), key: key.to_string(), ids: vec![] });
            out.len() - 1
        });
        if let (Some(id), Some(g)) = (id, out.get_mut(i)) {
            g.ids.push(id);
        }
    };
    d.walk(|n| {
        if let NodeKind::Image(im) = &n.kind
            && let Some(l) = &im.link
            && ids.is_none_or(|ids| ids.contains(&n.id))
        {
            add(l, &im.key, Some(n.id));
        }
    });
    if ids.is_none() {
        d.visit_images(|_, im| {
            if let Some(l) = &im.link {
                add(l, &im.key, None);
            }
        });
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
        None => fileio::raster_image(&bytes).is_ok_and(|img| img.key == g.key),
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

/// A linked file of a document (File → Package): its path, name and bytes.
pub(crate) struct LinkedFile {
    pub path: String,
    pub name: String,
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
            let bytes = locate(&g.link, folder(doc_path))
                .and_then(|mut f| f.read().ok())
                .or_else(|| d.images.get(&g.key).filter(own).map(|b| b.bytes.to_vec()));
            LinkedFile { path: g.link.path.clone(), name: g.link.name().to_string(), bytes }
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
/// when `o.clip` and the new art overflows the old bounds.
fn placed_xf(xf: Affine, old: (u32, u32), old_pt: (f64, f64), new: (u32, u32), new_pt: (f64, f64), o: PlacementOptions) -> (Affine, Option<Clip>) {
    let (ow, oh, nw, nh) = (old.0 as f64, old.1 as f64, new.0.max(1) as f64, new.1.max(1) as f64);
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
        let (xf, clip) = placed_xf(im.xf, (im.width, im.height), file_pt(d, &im.key), (img.width, img.height), new_pt, o);
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
            (Status::Modified, Some(mut f)) if update => match read_image(&mut f, &g.link).and_then(|read| install(d, &g.ids, read, false)) {
                Ok(()) => out.updated.extend(row),
                Err(_) => out.missing.extend(row),
            },
            (Status::Modified, _) => out.modified.extend(row),
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

/// Image objects `ids` link to `link`.
fn set_links(d: &mut Document, ids: &[NodeId], link: &LinkInfo) {
    for id in ids {
        if let Some(n) = d.node_mut(*id)
            && let NodeKind::Image(im) = &mut n.kind
        {
            im.link = Some(link.clone());
        }
    }
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
        if let NodeKind::Image(im) = &n.kind
            && let Some(l) = &im.link
        {
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
        if let Some(n) = d.node_mut(id)
            && let NodeKind::Image(im) = &mut n.kind
            && let Some(l) = &mut im.link
        {
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
        if previewing(&st.doc, &g.key) {
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
            (Status::Ok, Some(_)) if ids.is_none() && !previewing(&st.doc, &g.key) => {}
            // Unreadable as an image counts as missing.
            (_, Some(mut f)) => match read_image(&mut f, &g.link) {
                Ok(read) => reads.push((g.ids, read)),
                Err(_) => missing.extend(ids_json(&g.ids)),
            },
        }
    }
    let updated: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    if !reads.is_empty() {
        s.edit("Update Links", |d, _| reads.into_iter().try_for_each(|(ids, read)| install(d, &ids, read, false)))?;
    }
    Ok(json!({ "updated": updated, "missing": missing }))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.relink";
    let st = s.doc()?;
    let folder_param = str_param(p, "folder");
    let image = |id: &NodeId| st.doc.node(*id).and_then(|n| if let NodeKind::Image(im) = &n.kind { Some(im) } else { None });
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
        return Err(bad(C, format!("object {} is not an image", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(C, "select the images to relink, or pass ids"));
    }
    // (ids, the file) per file to read.
    let mut files: Vec<(Vec<NodeId>, String)> = vec![];
    let mut not_found = vec![];
    let mut also: Vec<NodeId> = vec![];
    match (str_param(p, "path"), folder_param) {
        (Some(path), _) => {
            let path = absolute_path(path);
            let mut ids = ids;
            // Every instance of the same linked file goes along.
            if bool_or(p, "allInstances", true) {
                let paths: Vec<&str> = ids.iter().filter_map(|id| image(id).and_then(|im| im.link.as_ref()).map(|l| l.path.as_str())).collect();
                let more: Vec<NodeId> = groups(&st.doc, None)
                    .into_iter()
                    .filter(|g| paths.contains(&g.link.path.as_str()))
                    .flat_map(|g| g.ids)
                    .filter(|id| !ids.contains(id))
                    .collect();
                ids.extend(more);
            }
            // Other missing files found in the new file's folder are relinked there too.
            if bool_or(p, "sameFolder", true)
                && let Some(dir) = Path::new(&path).parent()
            {
                let doc_dir = folder(st.path.as_deref());
                for g in groups(&st.doc, None) {
                    if g.ids.iter().any(|id| ids.contains(id)) || probe(&g, doc_dir).0 != Status::Missing {
                        continue;
                    }
                    let candidate = absolute_path(&dir.join(g.link.name()).to_string_lossy());
                    if candidate != path && file_stamp(&candidate).is_some() {
                        also.extend(g.ids.iter().copied());
                        match files.iter_mut().find(|(_, p)| *p == candidate) {
                            Some((v, _)) => v.extend(g.ids),
                            None => files.push((g.ids, candidate)),
                        }
                    }
                }
            }
            files.insert(0, (ids, path));
        }
        (None, Some(dir)) => {
            for id in ids {
                let Some(link) = image(&id).and_then(|im| im.link.as_ref()) else { continue };
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
        let img = fileio::raster_image(&bytes).map_err(|e| bad(C, format!("{path}: {e}")))?;
        reads.push((ids, (link_info(&path, &bytes), img)));
    }
    let relinked: Vec<u64> = reads.iter().flat_map(|(ids, _)| ids_json(ids)).collect();
    if !reads.is_empty() {
        s.edit("Relink", |d, _| reads.into_iter().try_for_each(|(ids, read)| install(d, &ids, read, false)))?;
    }
    not_found.sort();
    not_found.dedup();
    let mut out = json!({ "relinked": relinked, "notFound": not_found });
    if !also.is_empty() {
        out["alsoRelinked"] = json!(ids_json(&also));
    }
    Ok(out)
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

/// `ids` (else the selected images `keep` accepts), checked to be images; an error when none.
fn images_param(st: &crate::DocState, p: &Value, cmd: &str, keep: impl Fn(&ImageObject) -> bool) -> Result<Vec<NodeId>> {
    let ids = ids_param(p, "ids").unwrap_or_else(|| selected_images(st, keep));
    if let Some(id) = ids.iter().find(|id| image_of(&st.doc, **id).is_none()) {
        return Err(bad(cmd, format!("object {} is not an image", id.0)));
    }
    if ids.is_empty() {
        return Err(bad(cmd, "select the images, or pass ids"));
    }
    Ok(ids)
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

/// Per linked image in the layers of `st`: its file's status, where it was found when away from
/// its path, and whether the image shows its preview (each file probed once).
fn statuses(st: &crate::DocState) -> BTreeMap<NodeId, (Status, Option<String>, bool)> {
    let mut out = BTreeMap::new();
    for g in groups(&st.doc, None).iter().filter(|g| !g.ids.is_empty()) {
        let (status, found) = probe(g, folder(st.path.as_deref()));
        let found = found.map(|f| f.path).filter(|p| *p != g.link.path);
        let preview = previewing(&st.doc, &g.key);
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
    let mut images = vec![];
    st.doc.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            images.push((n, im));
        }
    });
    // Top first, as the Layers panel lists them.
    images.reverse();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rows = vec![];
    for (n, im) in images {
        let (status, found, preview) = match statuses.get(&n.id) {
            Some((s, f, p)) => (s.as_str(), f.as_deref(), *p),
            None => ("embedded", None, false),
        };
        *counts.entry(status).or_default() += 1;
        if show != "all" && show != status {
            continue;
        }
        let name = im.link.as_ref().map_or_else(|| n.display_name(), |l| l.name().to_string());
        let mut row = json!({
            "id": n.id.0,
            "name": name,
            "linked": im.link.is_some(),
            "status": status,
            "format": format_label(&st.doc, im),
            "pixelWidth": im.width,
            "pixelHeight": im.height,
        });
        if let Some(l) = &im.link {
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
    let id = image_param(st, p, "links.goTo")?;
    let b = st.doc.node(id).and_then(Node::geometric_bounds).unwrap_or_default();
    s.select(|_, sel| sel.set([id]))?;
    Ok(json!({ "id": id.0, "bounds": [b.x0, b.y0, b.x1, b.y1] }))
}

fn embed(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.embed";
    let st = s.doc()?;
    let ids = images_param(st, p, C, |im| im.link.is_some())?;
    // Images showing their preview take their file's pixels first.
    let (mut done, mut missing, mut reads) = (vec![], vec![], vec![]);
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
    if !done.is_empty() {
        s.edit("Embed", |d, _| {
            for (ids, read) in reads {
                install(d, &ids, read, true)?;
            }
            for id in &done {
                if let Some(n) = d.node_mut(*id)
                    && let NodeKind::Image(im) = &mut n.kind
                {
                    im.link = None;
                }
            }
            Ok(())
        })?;
    }
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

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.info";
    let id = image_param(s.doc()?, p, C)?;
    let mut out = super::place::image_info(s, &json!({ "id": id.0 }))?;
    let st = s.doc()?;
    let im = image_of(&st.doc, id).ok_or_else(|| bad(C, format!("object {} is not an image", id.0)))?;
    let (px, py) = file_pt(&st.doc, &im.key);
    let [a, b, c, d, ..] = im.xf.as_coeffs();
    out["effectivePpi"] = out["ppi"].take();
    out["ppi"] = json!([72.0 / px, 72.0 / py]);
    out["scale"] = json!([a.hypot(b) / px * 100.0, c.hypot(d) / py * 100.0]);
    // Counter-clockwise on the page (y points down).
    // Adding 0 turns -0 into 0.
    out["rotation"] = json!((-b).atan2(a).to_degrees() + 0.0);
    out["format"] = json!(format_label(&st.doc, im));
    out["placement"] = placement_json(im.placement);
    out["status"] = json!("embedded");
    if let Some(l) = &im.link {
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
    Ok(out)
}

fn placement_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "links.placementOptions";
    let st = s.doc()?;
    let ids = images_param(st, p, C, |_| true)?;
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
    let first = ids.first().and_then(|id| image_of(&st.doc, *id)).map(|im| im.placement).unwrap_or_default();
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
            if let Some(n) = d.node_mut(*id)
                && let NodeKind::Image(im) = &mut n.kind
            {
                set(&mut im.placement);
            }
        }
        Ok(())
    })?;
    let mut shown = first;
    set(&mut shown);
    Ok(json!({ "ids": ids_json(&ids), "placement": placement_json(shown) }))
}
