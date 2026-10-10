//! Linked files: where a placed file lives and what it was when last read ([`LinkInfo`]), the
//! low-resolution preview a linked image keeps for when its file can't be read
//! ([`ImageBlob::proxy`]), and every image object of a document ([`Document::visit_images`]).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{Document, ImageBlob, ImageObject, Node, NodeKind};

/// The longest side of a linked image's preview (pixels). Smaller images keep no preview: their
/// full pixels are saved instead.
pub const PROXY_SIZE: u32 = 256;
/// The MIME type of previews.
pub const PROXY_MIME: &str = "image/png";

/// A linked file: an image object shows the file at `path` (File → Place with Link) and the
/// document saves only a preview of it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LinkInfo {
    /// The file's absolute path.
    pub path: String,
    /// The path relative to the document's folder (`/` separators), written on save: how the file
    /// is found after the document and its links move together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<String>,
    /// The file's modification time when last read (milliseconds since the Unix epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,
    /// The file's size when last read (bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The hash of the file's bytes when last read ([`hash_bytes`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// The page shown, for a linked document page (1-based).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

impl LinkInfo {
    /// A link to `path` with nothing known about the file.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into(), ..Self::default() }
    }

    /// The file name: the path's last component, whichever system's separators it uses.
    pub fn name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }
}

impl From<&str> for LinkInfo {
    fn from(path: &str) -> Self {
        Self::new(path)
    }
}

/// [`ImageObject::link`] from a link object or, as files saved before links had details, a path.
pub(crate) fn de_link<'de, D: Deserializer<'de>>(d: D) -> Result<Option<LinkInfo>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Path(String),
        Info(LinkInfo),
    }
    Ok(Option::<Repr>::deserialize(d)?.map(|r| match r {
        Repr::Path(path) => LinkInfo::new(path),
        Repr::Info(info) => info,
    }))
}

/// A hash of `bytes` (FNV-1a), the same for identical files: `img` and 16 hex digits.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf29ce484222325u64, |h, x| (h ^ *x as u64).wrapping_mul(0x100000001b3));
    format!("img{h:016x}")
}

impl ImageBlob {
    /// A blob of encoded image `bytes` of type `mime`, without a preview.
    pub fn new(mime: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self { mime: mime.into(), bytes: Arc::new(bytes), proxy: None }
    }

    /// This blob with a preview of at most [`PROXY_SIZE`] pixels a side (PNG), when the image is
    /// larger than that and decodes; otherwise unchanged.
    pub fn with_proxy(mut self) -> Self {
        if self.proxy.is_none() {
            self.proxy = proxy_png(&self.bytes).map(Arc::new);
        }
        self
    }

    /// Are the bytes the preview (the linked file hasn't been read)?
    pub fn is_proxy(&self) -> bool {
        self.proxy.as_ref().is_some_and(|p| Arc::ptr_eq(p, &self.bytes))
    }
}

/// `bytes` decoded and shrunk to fit [`PROXY_SIZE`] as a PNG; `None` when small enough already or
/// undecodable.
fn proxy_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    if w.max(h) <= PROXY_SIZE {
        return None;
    }
    let small = image::load_from_memory(bytes).ok()?.thumbnail(PROXY_SIZE, PROXY_SIZE);
    let mut png = Vec::new();
    small.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
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

impl Document {
    /// Visit every image object: in the layers, then in symbol definitions and pattern swatches,
    /// opacity-mask art included.
    pub fn visit_images<'a>(&'a self, mut f: impl FnMut(&'a Node, &'a ImageObject)) {
        let roots = self.layers.iter().chain(self.symbols.iter().map(|s| &s.art)).chain(self.patterns.iter().flat_map(|p| &p.art));
        for root in roots {
            walk_all(root, &mut |n| {
                if let NodeKind::Image(im) = &n.kind {
                    f(n, im);
                }
            });
        }
    }

    /// The keys of image blobs only linked images (or placed documents, always linked) show: what a save writes as their preview.
    pub fn linked_only_images(&self) -> BTreeSet<String> {
        if self.images.values().all(|b| b.proxy.is_none()) {
            return BTreeSet::new();
        }
        // Per key: (shown linked, shown embedded).
        let mut uses: BTreeMap<&str, (bool, bool)> = BTreeMap::new();
        self.visit_images(|_, im| {
            let u = uses.entry(im.key.as_str()).or_default();
            if im.link.is_some() {
                u.0 = true;
            } else {
                u.1 = true;
            }
        });
        // A placed document saves its file's preview the same way.
        self.visit_placed(|_, p| uses.entry(p.key.as_str()).or_default().0 = true);
        uses.into_iter().filter(|(_, (linked, embedded))| *linked && !embedded).map(|(k, _)| k.to_string()).collect()
    }
}

/// Links panel → Placement Options: how a file read again (Relink, Update Link) takes the place of
/// the art it replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementOptions {
    #[serde(default)]
    pub preserve: Preserve,
    /// Where the new art sits in the old bounds (all but [`Preserve::Bounds`]).
    #[serde(default)]
    pub align: Align,
    /// Clip the new art to the old bounds where it is larger.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clip: bool,
}

/// What a relinked image keeps of the art it replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preserve {
    /// Its scale (relative to each file's 100% size), rotation and position.
    Transforms,
    /// Its bounds: the new art is stretched into them.
    #[default]
    Bounds,
    /// Nothing but its position: the new art at 100%, unrotated.
    FileDimensions,
    /// Its bounds: the new art scaled proportionally to fit inside them.
    Fit,
    /// Its bounds: the new art scaled proportionally to fill them.
    Fill,
}

impl Preserve {
    pub const ALL: [Preserve; 5] = [Preserve::Transforms, Preserve::Bounds, Preserve::FileDimensions, Preserve::Fit, Preserve::Fill];

    /// The id commands use (`transforms`, `bounds`, `fileDimensions`, `fit`, `fill`).
    pub fn id(self) -> &'static str {
        match self {
            Preserve::Transforms => "transforms",
            Preserve::Bounds => "bounds",
            Preserve::FileDimensions => "fileDimensions",
            Preserve::Fit => "fit",
            Preserve::Fill => "fill",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// A point of the 3×3 alignment grid, row by row from the top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Align {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Align {
    pub const ALL: [Align; 9] =
        [Align::TopLeft, Align::Top, Align::TopRight, Align::Left, Align::Center, Align::Right, Align::BottomLeft, Align::Bottom, Align::BottomRight];

    /// The id commands use (`topLeft` … `bottomRight`).
    pub fn id(self) -> &'static str {
        match self {
            Align::TopLeft => "topLeft",
            Align::Top => "top",
            Align::TopRight => "topRight",
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::BottomLeft => "bottomLeft",
            Align::Bottom => "bottom",
            Align::BottomRight => "bottomRight",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// The fractions (0, ½ or 1) of the free room left of and above the new art.
    pub fn fractions(self) -> (f64, f64) {
        let i = Self::ALL.iter().position(|a| *a == self).unwrap_or(4);
        ((i % 3) as f64 / 2.0, (i / 3) as f64 / 2.0)
    }
}

impl Document {
    /// Rewrite linked file locations in both raster images and placed VectorCraft documents,
    /// including those inside symbols, patterns and opacity masks. Used to make packages portable.
    pub fn update_links(&mut self, mut f: impl FnMut(&mut LinkInfo)) {
        fn walk(n: &mut Arc<Node>, f: &mut dyn FnMut(&mut LinkInfo)) {
            let n = Arc::make_mut(n);
            match &mut n.kind {
                NodeKind::Image(image) => {
                    if let Some(link) = &mut image.link {
                        f(link);
                    }
                }
                NodeKind::PlacedDocument(placed) => f(&mut placed.link),
                _ => {}
            }
            if let Some(mask) = &mut n.mask {
                walk(&mut mask.art, f);
            }
            for child in n.children_mut().into_iter().flatten() {
                walk(child, f);
            }
        }
        let roots =
            self.layers.iter_mut().chain(self.symbols.iter_mut().map(|s| &mut s.art)).chain(self.patterns.iter_mut().flat_map(|p| &mut p.art));
        for root in roots {
            walk(root, &mut f);
        }
    }

    /// Change every image object (in the layers, symbol definitions and pattern swatches, opacity-mask
    /// art included) for which `pick` holds; only the subtrees holding one are copied.
    pub fn update_images(&mut self, pick: impl Fn(&ImageObject) -> bool, mut f: impl FnMut(&mut ImageObject)) {
        fn holds(n: &Node, pick: &dyn Fn(&ImageObject) -> bool) -> bool {
            let mut hit = false;
            walk_all(n, &mut |c| hit |= matches!(&c.kind, NodeKind::Image(im) if pick(im)));
            hit
        }
        fn update(n: &mut Arc<Node>, pick: &dyn Fn(&ImageObject) -> bool, f: &mut dyn FnMut(&mut ImageObject)) {
            if !holds(n, pick) {
                return;
            }
            let n = Arc::make_mut(n);
            if let NodeKind::Image(im) = &mut n.kind
                && pick(im)
            {
                f(im);
            }
            if let Some(m) = &mut n.mask {
                update(&mut m.art, pick, f);
            }
            for c in n.children_mut().into_iter().flatten() {
                update(c, pick, f);
            }
        }
        let roots =
            self.layers.iter_mut().chain(self.symbols.iter_mut().map(|s| &mut s.art)).chain(self.patterns.iter_mut().flat_map(|p| &mut p.art));
        for root in roots {
            update(root, &pick, &mut f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeId;
    use vectorcraft_geom::Affine;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::from_pixel(w, h, image::Rgba([10, 200, 30, 255])).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn image(d: &mut Document, key: &str, link: Option<LinkInfo>) -> NodeId {
        let id = d.alloc_id();
        let layer = d.layers[0].id;
        d.insert(
            Some(layer),
            0,
            Node::new(
                id,
                NodeKind::Image(ImageObject { key: key.into(), width: 4, height: 4, xf: Affine::IDENTITY, link, placement: Default::default() }),
            ),
        )
        .unwrap()
    }

    #[test]
    fn a_path_string_from_older_files_reads_as_a_link() {
        let old: ImageObject = serde_json::from_str(r#"{"key":"k","width":2,"height":1,"xf":[1,0,0,1,0,0],"link":"C:\\art\\photo.png"}"#).unwrap();
        let link = old.link.unwrap();
        assert_eq!((link.path.as_str(), link.name(), link.hash.as_deref()), ("C:\\art\\photo.png", "photo.png", None));
        let new: ImageObject =
            serde_json::from_str(r#"{"key":"k","width":2,"height":1,"xf":[1,0,0,1,0,0],"link":{"path":"/a/b.png","size":9,"page":2}}"#).unwrap();
        assert_eq!(new.link, Some(LinkInfo { size: Some(9), page: Some(2), ..LinkInfo::new("/a/b.png") }));
        let none: ImageObject = serde_json::from_str(r#"{"key":"k","width":2,"height":1,"xf":[1,0,0,1,0,0]}"#).unwrap();
        assert_eq!(none.link, None);
        // Written as an object, with only what is known.
        let v = serde_json::to_value(&new).unwrap();
        assert_eq!(v["link"], serde_json::json!({"path": "/a/b.png", "size": 9, "page": 2}));
    }

    #[test]
    fn large_images_get_a_small_png_preview() {
        let big = ImageBlob::new("image/png", png(600, 300)).with_proxy();
        let p = image::load_from_memory(big.proxy.as_ref().unwrap()).unwrap();
        assert_eq!((p.width(), p.height()), (256, 128));
        assert!(!big.is_proxy(), "the bytes are the full image");
        assert!(ImageBlob::new("image/png", png(40, 20)).with_proxy().proxy.is_none(), "small images keep their pixels");
        assert!(ImageBlob::new("image/png", vec![1, 2, 3]).with_proxy().proxy.is_none(), "undecodable");
    }

    #[test]
    fn only_blobs_no_embedded_image_shows_are_linked_only() {
        let mut d = Document::new(100.0, 100.0);
        for k in ["a", "b", "c"] {
            d.images.insert(k.into(), ImageBlob::new("image/png", png(300, 10)).with_proxy());
        }
        image(&mut d, "a", Some(LinkInfo::new("/a.png")));
        image(&mut d, "b", Some(LinkInfo::new("/b.png")));
        image(&mut d, "b", None);
        image(&mut d, "c", None);
        assert_eq!(d.linked_only_images().into_iter().collect::<Vec<_>>(), ["a"]);
    }
}
