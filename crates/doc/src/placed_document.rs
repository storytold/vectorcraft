//! Placed documents: another VectorCraft file placed linked ([`PlacedDocument`]).
//!
//! The object keeps the link to the file and the file's bytes, stored like image data in
//! [`Document::images`] (with a small preview a save writes instead, as for linked images), so the
//! document shows correctly without the file. The art it shows is read from those bytes and never
//! joins the object tree, so editing can't reach it: it is locked by design. Drawing and exporting
//! expand it like a live object ([`crate::live::expanded`]).
//!
//! The file is read by the native format's loader, which lives above this crate and registers
//! itself ([`set_loader`]); the art read is kept in a small cache by content. Before exporting,
//! every exporter calls [`Document::with_placed_art`], which adds the resources the art uses
//! (images, symbols, patterns) under names of their own ([`RESOURCE_PREFIX`]) to a temporary copy
//! of the document, so they never meet the user's or show in panels. Placed documents inside a
//! placed document are read the same way, up to [`MAX_DEPTH`] deep (documents that place each
//! other stop there, showing their previews).

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use vectorcraft_geom::{Affine, Rect};

use vectorcraft_color::Paint;

use crate::{AppearanceItem, Document, ImageBlob, ImageObject, LinkInfo, Node, NodeKind, PatternDef, PlacementOptions, Symbol};

/// A placed document: an artboard of another VectorCraft file, shown as one locked object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacedDocument {
    /// The file (its `page` is the artboard shown, 1-based).
    pub link: LinkInfo,
    /// Key into [`Document::images`]: the file's bytes (see [`key_for`]).
    pub key: String,
    /// The art's bounds rather than the artboard (File › Place's `crop: "bounding"`).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub bounding: bool,
    /// The artboard's size (or the art's) at 100% (pt).
    pub width: f64,
    pub height: f64,
    /// Maps the box (0..width, 0..height) into the document.
    pub xf: Affine,
    /// Links panel → Placement Options: how the file read again takes this object's place.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub placement: PlacementOptions,
}

/// The images key of a file's bytes (`hash`, [`crate::links::hash_bytes`]) placed showing
/// artboard `page` (`bounding`: its art's bounds): objects showing the same thing share a blob and
/// its preview.
pub fn key_for(hash: &str, page: u32, bounding: bool) -> String {
    format!("{hash}-{page}{}", if bounding { "b" } else { "" })
}

impl PlacedDocument {
    /// The artboard shown (1-based).
    pub fn page(&self) -> u32 {
        self.link.page.unwrap_or(1)
    }

    /// The box at 100%.
    pub fn natural(&self) -> Rect {
        Rect::new(0.0, 0.0, self.width, self.height)
    }

    /// The four corners of the box in the document.
    pub fn corners(&self) -> [vectorcraft_geom::Point; 4] {
        let r = self.natural();
        [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| self.xf * vectorcraft_geom::Point::new(x, y))
    }

    /// The box in the document (axis-aligned).
    pub fn bounds(&self) -> Rect {
        self.xf.transform_rect_bbox(self.natural())
    }

    /// The transform from the file's coordinates (its artboard or art bounds `frame`) to the
    /// document; `None` when either size is empty.
    pub fn art_xf(&self, frame: Rect) -> Option<Affine> {
        let ok = self.width > 0.0 && self.height > 0.0 && frame.width() > 0.0 && frame.height() > 0.0;
        ok.then(|| {
            self.xf * Affine::scale_non_uniform(self.width / frame.width(), self.height / frame.height()) * Affine::translate((-frame.x0, -frame.y0))
        })
    }

    /// The art the object shows, as plain objects with `n`'s id: one group, its resources named
    /// as [`Document::with_placed_art`] adds them (nothing until that has read it).
    pub fn art(&self, n: &Node) -> Vec<Node> {
        let prefix = prefix(&self.key);
        read_cached(self).and_then(|e| self.art_node(n, &e, &prefix)).into_iter().collect()
    }

    /// Read `e`'s art placed as this object `n` shows it (`n`'s id), its resources renamed with
    /// `prefix`.
    fn art_node(&self, n: &Node, e: &ReadDocument, prefix: &str) -> Option<Node> {
        let m = self.art_xf(e.frame)?;
        let mut g = (*e.art).clone();
        rename(&mut g, prefix);
        g.transform(m, true);
        each_mut(&mut g, &mut |c| crate::pattern::transform_pattern_paints(c, m));
        g.id = n.id;
        Some(g)
    }

    /// An image object showing preview `bytes` (stored under this object's key) in this object's
    /// box: what it shows and outputs as when the file can't be read.
    pub fn preview_image(&self, bytes: &[u8]) -> Option<ImageObject> {
        let (w, h) = image_size(bytes).filter(|(w, h)| *w > 0 && *h > 0)?;
        let px = Affine::scale_non_uniform(self.width / w as f64, self.height / h as f64);
        Some(ImageObject { key: self.key.clone(), width: w, height: h, xf: self.xf * px, link: None, placement: Default::default() })
    }
}

/// The names the resources of the art stored under `key` take in a prepared document.
fn prefix(key: &str) -> String {
    format!("{RESOURCE_PREFIX}{key}/")
}

/// `n` and every descendant, opacity-mask art included.
fn walk_all<'a>(n: &'a Node, f: &mut impl FnMut(&'a Node)) {
    f(n);
    if let Some(m) = &n.mask {
        walk_all(&m.art, f);
    }
    for c in n.children().into_iter().flatten() {
        walk_all(c, f);
    }
}

/// The roots of `d`'s art: layers, symbol definitions and pattern swatches.
fn roots(d: &Document) -> impl Iterator<Item = &Arc<Node>> {
    d.layers.iter().chain(d.symbols.iter().map(|s| &s.art)).chain(d.patterns.iter().flat_map(|p| &p.art))
}

fn roots_mut(d: &mut Document) -> impl Iterator<Item = &mut Arc<Node>> {
    d.layers.iter_mut().chain(d.symbols.iter_mut().map(|s| &mut s.art)).chain(d.patterns.iter_mut().flat_map(|p| &mut p.art))
}

impl Document {
    /// Does the document hold a compound shape anywhere (layers, symbols, patterns, masks)?
    pub fn has_compound_shapes(&self) -> bool {
        let mut any = false;
        for root in roots(self) {
            walk_all(root, &mut |n| any |= matches!(n.kind, NodeKind::CompoundShape { .. }));
        }
        any
    }

    /// Every compound shape (the outermost of nested ones) replaced by `art(compound)`, for apps
    /// that don't know them. Only the subtrees holding one are copied.
    pub fn compound_shapes_as(&mut self, art: &mut dyn FnMut(&Node) -> Node) {
        fn holds(n: &Node) -> bool {
            let mut hit = false;
            walk_all(n, &mut |c| hit |= matches!(c.kind, NodeKind::CompoundShape { .. }));
            hit
        }
        fn update(n: &mut Arc<Node>, art: &mut dyn FnMut(&Node) -> Node) {
            if !holds(n) {
                return;
            }
            if matches!(n.kind, NodeKind::CompoundShape { .. }) {
                *n = Arc::new(art(n));
            }
            let n = Arc::make_mut(n);
            if let Some(m) = &mut n.mask {
                update(&mut m.art, art);
            }
            for c in n.children_mut().into_iter().flatten() {
                update(c, art);
            }
        }
        for root in roots_mut(self) {
            update(root, art);
        }
    }
}

impl Document {
    /// Visit every placed document: in the layers, then in symbol definitions and pattern
    /// swatches, opacity-mask art included.
    pub fn visit_placed<'a>(&'a self, mut f: impl FnMut(&'a Node, &'a PlacedDocument)) {
        for root in roots(self) {
            walk_all(root, &mut |n| {
                if let NodeKind::PlacedDocument(p) = &n.kind {
                    f(n, p);
                }
            });
        }
    }

    /// Does the document hold a placed document anywhere?
    pub fn has_placed(&self) -> bool {
        let mut any = false;
        self.visit_placed(|_, _| any = true);
        any
    }

    /// Every placed document (those in the art they show too) replaced by a plain group of that
    /// art, for apps that don't know them: call it on [`Self::with_placed_art`]'s copy. Only the
    /// subtrees holding one are copied.
    pub fn placed_as_groups(&mut self) {
        fn holds(n: &Node) -> bool {
            let mut hit = false;
            walk_all(n, &mut |c| hit |= matches!(c.kind, NodeKind::PlacedDocument(_)));
            hit
        }
        fn update(n: &mut Arc<Node>) {
            if !holds(n) {
                return;
            }
            if matches!(n.kind, NodeKind::PlacedDocument(_)) {
                let mut g = crate::live::expanded_group(n, None);
                g.appearance = n.appearance.clone();
                *n = Arc::new(g);
            }
            // The art shown may hold placed documents too.
            let n = Arc::make_mut(n);
            if let Some(m) = &mut n.mask {
                update(&mut m.art);
            }
            for c in n.children_mut().into_iter().flatten() {
                update(c);
            }
        }
        for root in roots_mut(self) {
            update(root);
        }
    }

    /// Placed documents whose key is in `keys` replaced by images of their previews (for output
    /// when their files can't be read).
    pub fn placed_as_previews(&mut self, keys: &BTreeSet<String>) {
        fn update(n: &mut Arc<Node>, keys: &BTreeSet<String>, images: &std::collections::BTreeMap<String, ImageBlob>) {
            let mut hit = false;
            walk_all(n, &mut |c| hit |= matches!(&c.kind, NodeKind::PlacedDocument(p) if keys.contains(&p.key)));
            if !hit {
                return;
            }
            let n = Arc::make_mut(n);
            if let NodeKind::PlacedDocument(p) = &n.kind
                && keys.contains(&p.key)
            {
                if let Some(im) = images.get(&p.key).and_then(|b| p.preview_image(&b.bytes)) {
                    n.kind = NodeKind::Image(im);
                }
                return;
            }
            if let Some(m) = &mut n.mask {
                update(&mut m.art, keys, images);
            }
            for c in n.children_mut().into_iter().flatten() {
                update(c, keys, images);
            }
        }
        let images = self.images.clone();
        for root in roots_mut(self) {
            update(root, keys, &images);
        }
    }
}

// ---------- previews and the files' bytes ----------

/// A placed document's preview: this many pixels per point of its box…
pub const PREVIEW_PX_PER_PT: f64 = 1.0;
/// …and at most this many on its longer side.
pub const PREVIEW_MAX: u32 = 1024;
/// How deep placed documents inside placed documents are read; deeper ones show their previews.
pub const MAX_DEPTH: usize = 8;

/// Reads `p`'s file again (`None`: not found, or changed since it was read). The engine registers
/// one with [`set_file_reader`].
pub type FileReader = fn(&PlacedDocument) -> Option<Vec<u8>>;

static FILE_READER: OnceLock<FileReader> = OnceLock::new();

/// Register how files are read again (the first registration stays).
pub fn set_file_reader(f: FileReader) {
    // Already set: the same engine, registered by another session.
    let _ = FILE_READER.set(f);
}

/// The most bytes of files read again kept.
const FULL_BYTES: usize = 512 << 20;

/// Files read again, by key, the most recently read last.
type FullFiles = Vec<(String, Arc<Vec<u8>>)>;

static FULL: Mutex<FullFiles> = Mutex::new(Vec::new());

fn full() -> MutexGuard<'static, FullFiles> {
    // Entries are only pushed and removed: a panic while held leaves it whole.
    FULL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The file's bytes when they are at hand: stored (`blob` isn't just the preview) or read again
/// already. Never reads it.
pub fn full_bytes_cached(p: &PlacedDocument, blob: &ImageBlob) -> Option<Arc<Vec<u8>>> {
    if !blob.is_proxy() {
        return Some(blob.bytes.clone());
    }
    full().iter().rev().find(|(k, _)| *k == p.key).map(|(_, b)| b.clone())
}

/// The file's bytes: stored, read again already, or read again now ([`set_file_reader`]). `None`
/// when only the preview can be had.
pub fn full_bytes(p: &PlacedDocument, blob: &ImageBlob) -> Option<Arc<Vec<u8>>> {
    if let Some(b) = full_bytes_cached(p, blob) {
        return Some(b);
    }
    let b = Arc::new(FILE_READER.get()?(p)?);
    let mut c = full();
    c.push((p.key.clone(), b.clone()));
    let mut total: usize = c.iter().map(|(_, b)| b.len()).sum();
    while total > FULL_BYTES && c.len() > 1 {
        total = total.saturating_sub(c.remove(0).1.len());
    }
    Some(b)
}

/// The size in pixels of image bytes `bytes` (a preview), read from its header.
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()
}

// ---------- reading files ----------

/// The prefix of the names the resources of placed documents' art take in a prepared document:
/// `placed:<key>/<name>`.
pub const RESOURCE_PREFIX: &str = "placed:";

/// Reads a placed document's file (its bytes) into what it shows: a document holding the
/// resources (no layers), the art as one group in the file's coordinates, and the frame shown
/// (the artboard, or the art's bounds). The native format lives above this crate, so the engine
/// registers one with [`set_loader`].
pub type Loader = fn(&PlacedDocument, &[u8]) -> Option<(Document, Node, Rect)>;

static LOADER: OnceLock<Loader> = OnceLock::new();

/// Register how files are read (the first registration stays).
pub fn set_loader(l: Loader) {
    // Already set: the same loader, registered by another session.
    let _ = LOADER.set(l);
}

/// The most files kept read whatever their size.
const CACHE_SIZE: usize = 64;
/// Past [`CACHE_SIZE`], the most bytes of files kept read (their own sizes, standing for what
/// reading them holds).
const CACHE_BYTES: usize = 256 << 20;

/// A placed document's file read: the art it shows, ready to draw with its own resources.
pub struct ReadDocument {
    key: String,
    /// Read from the preview, not the file.
    preview: bool,
    /// The bytes read (what tells a cached read is still the one shown).
    bytes: Arc<Vec<u8>>,
    /// The artboard (or art bounds) shown: what the object's box shows.
    pub frame: Rect,
    /// The art as one group, in the file's coordinates. Its images, symbols and patterns are
    /// `doc`'s, under their own names.
    pub art: Arc<Node>,
    /// The file's resources and setup (no layers: the art is `art`).
    pub doc: Arc<Document>,
}

struct Entry {
    e: Arc<ReadDocument>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    entries: std::collections::HashMap<(String, bool), Entry>,
    clock: u64,
}

static CACHE: std::sync::LazyLock<Mutex<Cache>> = std::sync::LazyLock::new(Default::default);

fn cache() -> MutexGuard<'static, Cache> {
    // A panic while the cache was held leaves it whole (entries are only inserted and removed).
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Cache {
    /// The read of `key` (its preview's when `preview`; made from `bytes` when given), marked used.
    fn get(&mut self, key: &str, preview: bool, bytes: Option<&Arc<Vec<u8>>>) -> Option<Arc<ReadDocument>> {
        self.clock += 1;
        let clock = self.clock;
        let e = self.entries.get_mut(&(key.to_string(), preview))?;
        if bytes.is_some_and(|b| !Arc::ptr_eq(&e.e.bytes, b)) {
            return None;
        }
        e.used = clock;
        Some(e.e.clone())
    }

    fn insert(&mut self, e: Arc<ReadDocument>) {
        self.clock += 1;
        self.entries.insert((e.key.clone(), e.preview), Entry { e, used: self.clock });
        // The least recently used go while over both limits.
        let mut total: usize = self.entries.values().map(|x| x.e.bytes.len()).sum();
        while self.entries.len() > CACHE_SIZE && total > CACHE_BYTES {
            let Some(k) = self.entries.iter().min_by_key(|(_, x)| x.used).map(|(k, _)| k.clone()) else { break };
            if let Some(x) = self.entries.remove(&k) {
                total = total.saturating_sub(x.e.bytes.len());
            }
        }
    }
}

/// `p`'s file (`bytes`) read, from the cache when it was read already: what the renderer draws a
/// placed document from. `None` when it can't be read (or no loader is registered).
pub fn read(p: &PlacedDocument, bytes: &Arc<Vec<u8>>) -> Option<Arc<ReadDocument>> {
    if let Some(e) = cache().get(&p.key, false, Some(bytes)) {
        return Some(e);
    }
    let loader = LOADER.get()?;
    let (doc, art, frame) = loader(p, bytes)?;
    let e = Arc::new(ReadDocument { key: p.key.clone(), preview: false, bytes: bytes.clone(), frame, art: Arc::new(art), doc: Arc::new(doc) });
    cache().insert(e.clone());
    Some(e)
}

/// `p`'s preview (`bytes`) read as art: one image filling the box.
fn read_preview(p: &PlacedDocument, bytes: &Arc<Vec<u8>>) -> Option<Arc<ReadDocument>> {
    if let Some(e) = cache().get(&p.key, true, Some(bytes)) {
        return Some(e);
    }
    let (w, h) = image_size(bytes).filter(|(w, h)| *w > 0 && *h > 0)?;
    let frame = p.natural();
    let mut doc = Document::new(frame.width().max(1.0), frame.height().max(1.0));
    doc.layers.clear();
    doc.swatches.clear();
    let mime = if bytes.starts_with(&[0xff, 0xd8]) { "image/jpeg" } else { "image/png" };
    doc.images.insert("preview".into(), ImageBlob { mime: mime.into(), bytes: bytes.clone(), proxy: None });
    let xf = Affine::scale_non_uniform(frame.width() / w as f64, frame.height() / h as f64);
    let im = ImageObject { key: "preview".into(), width: w, height: h, xf, link: None, placement: Default::default() };
    let art = Node::new(art_id(), NodeKind::Group { children: vec![Arc::new(Node::new(art_id(), NodeKind::Image(im)))], clip: false });
    let e = Arc::new(ReadDocument { key: p.key.clone(), preview: true, bytes: bytes.clone(), frame, art: Arc::new(art), doc: Arc::new(doc) });
    cache().insert(e.clone());
    Some(e)
}

/// `p`'s file as read already (else its preview's), without reading it.
pub fn read_cached(p: &PlacedDocument) -> Option<Arc<ReadDocument>> {
    let mut c = cache();
    c.get(&p.key, false, None).or_else(|| c.get(&p.key, true, None))
}

/// Keep `read` (what [`Loader`] gives), just read from `bytes` for `p` (when it was placed), so
/// drawing it doesn't read it again.
pub fn prime(p: &PlacedDocument, bytes: Arc<Vec<u8>>, (doc, art, frame): (Document, Node, Rect)) {
    cache().insert(Arc::new(ReadDocument { key: p.key.clone(), preview: false, bytes, frame, art: Arc::new(art), doc: Arc::new(doc) }));
}

/// The id the read art's own nodes take (their object's, once expanded).
fn art_id() -> crate::NodeId {
    crate::NodeId(0)
}

/// Visit `n` and every descendant (opacity-mask art included) mutably.
fn each_mut(n: &mut Node, f: &mut impl FnMut(&mut Node)) {
    f(n);
    if let Some(m) = &mut n.mask {
        each_mut(Arc::make_mut(&mut m.art), f);
    }
    for c in n.children_mut().into_iter().flatten() {
        each_mut(Arc::make_mut(c), f);
    }
}

/// `n`'s references to images, symbols, patterns and placed files prefixed with `prefix`. Colours
/// keep their swatch links (a spot colour stays a plate, one with the parent's spot of that name).
fn rename(n: &mut Node, prefix: &str) {
    let paint = |p: &mut Paint| {
        if let Paint::Pattern { pattern, .. } = p {
            *pattern = format!("{prefix}{pattern}");
        }
    };
    each_mut(n, &mut |n| {
        for it in &mut n.appearance.items {
            match it {
                AppearanceItem::Fill(f) => paint(&mut f.paint),
                AppearanceItem::Stroke(s) => paint(&mut s.paint),
            }
        }
        match &mut n.kind {
            NodeKind::Text(t) => {
                for r in &mut t.runs {
                    paint(&mut r.style.fill);
                    paint(&mut r.style.stroke);
                }
            }
            NodeKind::Image(im) => im.key = format!("{prefix}{}", im.key),
            NodeKind::PlacedDocument(p) => p.key = format!("{prefix}{}", p.key),
            NodeKind::SymbolInstance { symbol, .. } => *symbol = format!("{prefix}{symbol}"),
            _ => {}
        }
    });
}

impl Document {
    /// This document ready to export its placed documents: their files read (the file read again
    /// when only the preview is stored, else the preview), and the resources their art uses added
    /// under names of their own ([`RESOURCE_PREFIX`]); the same for the placed documents in that
    /// art, up to [`MAX_DEPTH`] deep. Borrowed when there is none. (The renderer draws them from
    /// [`read`] instead.)
    pub fn with_placed_art(&self) -> Cow<'_, Document> {
        let mut queue: Vec<(PlacedDocument, usize)> = vec![];
        let mut seen = BTreeSet::new();
        self.visit_placed(|_, p| {
            if seen.insert(p.key.clone()) {
                queue.push((p.clone(), 0));
            }
        });
        if queue.is_empty() {
            return Cow::Borrowed(self);
        }
        let mut d = self.clone();
        let mut found: Vec<Arc<ReadDocument>> = vec![];
        while let Some((p, depth)) = queue.pop() {
            let Some(blob) = d.images.get(&p.key).cloned() else { continue };
            let full = if depth < MAX_DEPTH { full_bytes(&p, &blob) } else { None };
            if full.and_then(|b| read(&p, &b)).is_none() {
                let preview = if blob.is_proxy() { Some(blob.bytes.clone()) } else { blob.proxy.clone() };
                if let Some(bytes) = preview {
                    read_preview(&p, &bytes);
                }
            }
            // What the art (`PlacedDocument::art`) will show.
            let Some(e) = read_cached(&p) else { continue };
            let prefix = prefix(&p.key);
            add_resources(&mut d, &e.doc, &prefix);
            // The placed documents in that art, under their names here.
            let inner = std::iter::once(&e.art).chain(e.doc.symbols.iter().map(|s| &s.art)).chain(e.doc.patterns.iter().flat_map(|p| &p.art));
            for root in inner {
                walk_all(root, &mut |n| {
                    if let NodeKind::PlacedDocument(q) = &n.kind {
                        let key = format!("{prefix}{}", q.key);
                        if seen.insert(key.clone()) {
                            queue.push((PlacedDocument { key, ..(**q).clone() }, depth + 1));
                        }
                    }
                });
            }
            found.push(e);
        }
        // Read last, so all stay read while the art is made (see `PlacedDocument::art`).
        {
            let mut c = cache();
            for e in &found {
                c.get(&e.key, e.preview, None);
            }
        }
        Cow::Owned(d)
    }

    /// Drop the resources [`Self::with_placed_art`] added under its names (what a save leaves out).
    pub fn drop_placed_resources(&mut self) {
        self.symbols.retain(|s| !s.name.starts_with(RESOURCE_PREFIX));
        self.patterns.retain(|p| !p.name.starts_with(RESOURCE_PREFIX));
        self.images.retain(|k, _| !k.starts_with(RESOURCE_PREFIX));
    }
}

/// The resources of `src` (a read file's) added to `d` under `prefix`: symbols, patterns, images;
/// swatches by name, when `d` has none of that name.
fn add_resources(d: &mut Document, src: &Document, prefix: &str) {
    for s in &src.symbols {
        let name = format!("{prefix}{}", s.name);
        if !d.symbols.iter().any(|x| x.name == name) {
            let mut s = s.clone();
            rename(Arc::make_mut(&mut s.art), prefix);
            d.symbols.push(Symbol { name, ..s });
        }
    }
    for p in &src.patterns {
        let name = format!("{prefix}{}", p.name);
        if d.pattern(&name).is_none() {
            let mut p = p.clone();
            p.art.iter_mut().for_each(|a| rename(Arc::make_mut(a), prefix));
            d.patterns.push(PatternDef { name, ..p });
        }
    }
    for (k, b) in &src.images {
        d.images.entry(format!("{prefix}{k}")).or_insert_with(|| b.clone());
    }
    for w in &src.swatches {
        if d.swatch(&w.name).is_none() {
            d.swatches.push(w.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeId;

    fn placed(key: &str) -> PlacedDocument {
        PlacedDocument {
            link: LinkInfo::new("/art/logo.vectorcraft"),
            key: key.into(),
            bounding: false,
            width: 80.0,
            height: 40.0,
            xf: Affine::translate((10.0, 10.0)),
            placement: Default::default(),
        }
    }

    /// Whatever the bytes: a 40×20 artboard showing a symbol instance and, when the bytes say
    /// `nest`, a placed document of key `inner`.
    fn stub_loader(_: &PlacedDocument, bytes: &[u8]) -> Option<(Document, Node, Rect)> {
        let mut d = Document::new(40.0, 20.0);
        d.layers.clear();
        let art = Node::new(
            NodeId(50),
            NodeKind::Image(ImageObject { key: "pic".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default() }),
        );
        d.symbols.push(Symbol { name: "Star".into(), art: Arc::new(art) });
        d.images.insert("pic".into(), ImageBlob::new("image/png", vec![1, 2, 3]));
        let mut children = vec![Arc::new(Node::new(NodeId(51), NodeKind::SymbolInstance { symbol: "Star".into(), xf: Affine::IDENTITY }))];
        if bytes.starts_with(b"nest") {
            d.images.insert("inner".into(), ImageBlob::new("application/json", b"leaf".to_vec()));
            let inner = PlacedDocument { key: "inner".into(), width: 10.0, height: 10.0, xf: Affine::IDENTITY, ..placed("inner") };
            children.push(Arc::new(Node::new(NodeId(52), NodeKind::PlacedDocument(Box::new(inner)))));
        }
        Some((d, Node::new(NodeId(0), NodeKind::Group { children, clip: false }), Rect::new(0.0, 0.0, 40.0, 20.0)))
    }

    #[test]
    fn round_trips_through_json() {
        let n = Node::new(NodeId(3), NodeKind::PlacedDocument(Box::new(placed("doc1-1"))));
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["kind"]["type"], "placeddocument");
        assert_eq!(v["kind"]["link"]["path"], "/art/logo.vectorcraft");
        let back: Node = serde_json::from_value(v).unwrap();
        assert_eq!(back, n);
        assert_eq!(key_for("img00", 2, true), "img00-2b");
    }

    #[test]
    fn the_art_and_its_resources_join_only_a_prepared_copy_under_their_own_names() {
        set_loader(stub_loader);
        let mut d = Document::new(200.0, 200.0);
        d.symbols.push(Symbol { name: "Star".into(), art: Arc::new(Node::new(NodeId(60), NodeKind::Group { children: vec![], clip: false })) });
        d.images.insert("doc1-1".into(), ImageBlob::new("application/json", b"nest".to_vec()));
        let p = placed("doc1-1");
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, Node::new(NodeId(7), NodeKind::PlacedDocument(Box::new(p.clone())))).unwrap();
        assert!(d.has_placed());
        let prepared = d.with_placed_art();
        let names: Vec<&str> = prepared.symbols.iter().map(|s| s.name.as_str()).collect();
        // The placed document inside is read too, its resources under its own names.
        assert_eq!(names, ["Star", "placed:doc1-1/Star", "placed:placed:doc1-1/inner/Star"]);
        assert!(prepared.images.contains_key("placed:doc1-1/pic"));
        assert_eq!(d.symbols.len(), 1, "the document itself is unchanged");
        // The art: a group of the drawing, its resources renamed.
        let n = d.node(NodeId(7)).unwrap();
        let art = p.art(n);
        let [g] = &art[..] else { panic!("{art:?}") };
        let mut refs = vec![];
        g.walk(&mut |c| match &c.kind {
            NodeKind::SymbolInstance { symbol, .. } => refs.push(symbol.clone()),
            NodeKind::PlacedDocument(q) => {
                // Its 10×10 box at the file's origin, scaled ×2 into the box at (10, 10).
                assert_eq!(q.bounds(), Rect::new(10.0, 10.0, 30.0, 30.0));
                refs.push(q.key.clone())
            }
            _ => {}
        });
        assert_eq!(refs, ["placed:doc1-1/Star", "placed:doc1-1/inner"]);
        // For older apps: plain groups all the way down.
        let mut older = prepared.clone().into_owned();
        older.placed_as_groups();
        assert!(!older.has_placed());
        let mut saved = prepared.into_owned();
        saved.drop_placed_resources();
        assert_eq!((saved.symbols.len(), saved.images.len()), (1, 1));
    }

    #[test]
    fn more_files_than_the_cache_holds_all_draw() {
        set_loader(stub_loader);
        let mut d = Document::new(200.0, 200.0);
        let layer = d.layers[0].id;
        let n = CACHE_SIZE + 10;
        for i in 0..n {
            let key = format!("many{i}-1");
            d.images.insert(key.clone(), ImageBlob::new("application/json", format!("doc {i}").into_bytes()));
            d.insert(Some(layer), i, Node::new(NodeId(100 + i as u64), NodeKind::PlacedDocument(Box::new(placed(&key))))).unwrap();
        }
        let _prepared = d.with_placed_art();
        let mut empty = 0;
        d.visit_placed(|n, p| empty += usize::from(p.art(n).is_empty()));
        assert_eq!(empty, 0, "every file of the document is kept read");
    }

    #[test]
    fn a_file_that_cant_be_read_shows_its_preview() {
        set_loader(stub_loader);
        let mut png = vec![];
        image::RgbaImage::from_pixel(8, 4, image::Rgba([0, 200, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let mut blob = ImageBlob::new("image/png", png);
        blob.proxy = Some(blob.bytes.clone());
        let mut d = Document::new(200.0, 200.0);
        d.images.insert("gone-1".into(), blob);
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, Node::new(NodeId(7), NodeKind::PlacedDocument(Box::new(placed("gone-1"))))).unwrap();
        // No file reader: only the preview can be had.
        let prepared = d.with_placed_art();
        assert!(prepared.images.contains_key("placed:gone-1/preview"));
        let n = d.node(NodeId(7)).unwrap();
        let NodeKind::PlacedDocument(p) = &n.kind else { panic!("not a placed document") };
        let art = p.art(n);
        assert_eq!(art.first().and_then(Node::geometric_bounds), Some(Rect::new(10.0, 10.0, 90.0, 50.0)));
    }
}
