//! Files `<image>` elements link to (`href="photo.png"`, `file:///…`): read before usvg runs,
//! through [`ImportOptions::read`], relative paths from [`ImportOptions::folder`]. A raster file
//! stays a linked image ([`LinkInfo`]); an SVG file becomes vector art. One that can't be read
//! (missing, remote, not an image) becomes a placeholder that keeps the link and its place, so it
//! can be relinked, and a warning.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use usvg::{ImageHrefResolver, ImageKind, roxmltree};
use vectorcraft_doc::LinkInfo;

use super::{Edits, depth, href};
use crate::{ImportOptions, MAX_ELEMENT_DEPTH, MAX_SVGZ, SvgError};

/// Most nested SVG images an imported SVG may decode (#3564).
pub const MAX_NESTED_SVG_IMAGES: usize = 256;

/// The files an SVG's images link to.
#[derive(Default)]
pub(in crate::import) struct Files {
    /// What usvg gets for each linked href.
    sources: HashMap<String, Source>,
    /// The link of each linked raster or placeholder, by the address of the bytes usvg got.
    links: HashMap<usize, Linked>,
    /// An SVG image had text (which isn't imported).
    text: AtomicBool,
    /// How deep the SVGs around the SVG image being read nest, added up: usvg reads an SVG image
    /// while it reads the SVG showing it, on the same stack, so one cap ([`MAX_ELEMENT_DEPTH`])
    /// holds for them all.
    depth: AtomicUsize,
    /// The bytes the SVGZ images (data and files) unpacked to so far, whether or not they
    /// decoded: together they unpack to at most what one SVGZ file may ([`MAX_SVGZ`]), however
    /// they nest.
    unpacked: AtomicU64,
    /// An SVG image was left out for nesting too deep in the SVGs around it.
    too_deep: AtomicBool,
    /// An SVGZ image was left out for unpacking past what the SVGZ images may.
    too_big: AtomicBool,
    /// How many nested SVG images were decoded so far.
    svg_images: AtomicUsize,
    /// SVG images were left out for exceeding [`MAX_NESTED_SVG_IMAGES`].
    too_many_svg_images: AtomicBool,
}

/// How usvg holds the bytes of a raster of one format (`ImageKind::PNG`…).
type RasterKind = fn(Arc<Vec<u8>>) -> ImageKind;

enum Source {
    /// A raster file of the kind usvg reads it as.
    Raster(RasterKind, Arc<Vec<u8>>),
    /// An SVG file's text (unpacked when it is SVGZ), parsed by usvg as an image.
    Svg(String),
}

/// A linked raster image (or the placeholder of a missing one).
pub(in crate::import) struct Linked {
    pub link: LinkInfo,
    /// The file wasn't read: the bytes are a placeholder.
    pub missing: bool,
}

/// The side of the placeholder image, in pixels.
const MARKER: u32 = 64;

/// The placeholder of a missing linked image: a light tile, crossed out (drawn here).
fn marker() -> Option<Vec<u8>> {
    let last = MARKER - 1;
    let img = image::RgbaImage::from_fn(MARKER, MARKER, |x, y| {
        let edge = x == 0 || y == 0 || x == last || y == last;
        let cross = x.abs_diff(y) <= 1 || (x + y).abs_diff(last) <= 1;
        image::Rgba(if edge || cross { [200, 60, 60, 255] } else { [236, 236, 236, 255] })
    });
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
}

/// `s` with `%XX` escapes decoded (an href is a URL).
fn percent_decoded(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = tail
            .get(..2)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(v) if b == b'%' => {
                out.push(v);
                rest = tail.get(2..).unwrap_or_default();
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where href `h` points: the path its link keeps, and the file to read when it may be read (a
/// local file, or one on the network share the SVG is on). A relative path is taken from
/// `folder`, the SVG's own, made absolute against the working directory when it isn't; without
/// a folder, only absolute paths are read (see `craft_docpath`). The link keeps the path as
/// written (decoded) when the folder does not change the path it resolves to, and the path found
/// when it does (a relative path, or on Windows a path rooted without a drive); the link of a
/// `file:` URL keeps the path it names.
fn target(h: &str, folder: Option<&str>) -> (String, Option<String>) {
    let h = h.trim();
    // `craft_docpath` decodes a `file:` URL itself; any other href is decoded here, once.
    let file_url = h.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("file:"));
    let stored = if file_url { h.to_string() } else { percent_decoded(h) };
    if !file_url && stored.contains("://") {
        return (stored, None);
    }
    // Resolved as a document inside `folder`: only the folder is used.
    let document = folder.map(|f| {
        let f = Path::new(f);
        if f.is_absolute() { f.to_path_buf() } else { std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf()) }.join("-")
    });
    match craft_docpath::resolve(&stored, document.as_deref()) {
        Ok(path) => {
            let file = craft_docpath::resolve_absolute(&stored, document.as_deref()).ok().map(|p| p.to_string_lossy().into_owned());
            let as_written = !file_url && craft_docpath::resolve(&stored, None).as_ref() == Ok(&path);
            let link = if as_written { stored } else { path.to_string_lossy().into_owned() };
            (link, file)
        }
        Err(_) => (stored, None),
    }
}

/// The address of image bytes, to find their link again.
fn key(bytes: &Arc<Vec<u8>>) -> usize {
    Arc::as_ptr(bytes) as usize
}

impl Files {
    /// Read the files the `<image>` elements of `xml` (the text `svg`) link to. Placeholders
    /// stretch to the image's box (`preserveAspectRatio="none"`), which the file fills once
    /// relinked.
    pub(in crate::import) fn read(svg: &str, xml: &roxmltree::Document, opts: &ImportOptions, edits: &mut Edits, warnings: &mut Vec<String>) -> Self {
        let mut files = Files::default();
        let mut placeholder = None;
        for n in xml.descendants().filter(|n| n.is_element() && n.tag_name().name() == "image") {
            let Some(h) = href(n).filter(|h| !h.trim_start().starts_with("data:")) else { continue };
            if !files.sources.contains_key(h) {
                let (path, file) = target(h, opts.folder);
                let read = file.and_then(|file| opts.read.and_then(|read| read(&file)));
                let source = match read {
                    Some((bytes, link)) => files.source(bytes, link).ok_or("is not an image VectorCraft reads"),
                    None => Err("not found"),
                };
                let source = source.or_else(|why| {
                    warnings.push(format!("linked image '{h}' {why}: a placeholder keeps its place until it is relinked"));
                    let bytes = Arc::new(placeholder.get_or_insert_with(marker).clone().ok_or(why)?);
                    files.links.insert(key(&bytes), Linked { link: LinkInfo::new(path), missing: true });
                    Ok::<_, &str>(Source::Raster(ImageKind::PNG, bytes))
                });
                let Ok(source) = source else { continue };
                files.sources.insert(h.to_string(), source);
            }
            if files.placeholder(h) {
                edits.set(svg, n, "preserveAspectRatio", "none");
            }
        }
        files
    }

    /// What usvg gets for a file read (`bytes`, linked by `link`): a raster usvg reads (others are
    /// stored as PNG) or an SVG; `None` for anything else.
    fn source(&mut self, bytes: Vec<u8>, link: LinkInfo) -> Option<Source> {
        let Ok(format) = image::guess_format(&bytes) else {
            // Unpacked once, here, within what the SVGZ images may unpack to.
            let text = crate::text_within(&bytes, &self.unpacked).ok()?;
            return text.contains("<svg").then(|| Source::Svg(text.into_owned()));
        };
        let Some(kind) = raster(format) else {
            let img = image::load_from_memory_with_format(&bytes, format).ok()?;
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            let png = Arc::new(png);
            self.links.insert(key(&png), Linked { link, missing: false });
            return Some(Source::Raster(ImageKind::PNG, png));
        };
        let bytes = Arc::new(bytes);
        self.links.insert(key(&bytes), Linked { link, missing: false });
        Some(Source::Raster(kind, bytes))
    }

    /// Does href `h` show a placeholder?
    fn placeholder(&self, h: &str) -> bool {
        matches!(self.sources.get(h), Some(Source::Raster(_, b)) if self.links.get(&key(b)).is_some_and(|l| l.missing))
    }

    /// The link of the image whose data usvg got as `kind`.
    pub(in crate::import) fn linked(&self, kind: &ImageKind) -> Option<&Linked> {
        match kind {
            ImageKind::PNG(b) | ImageKind::JPEG(b) | ImageKind::GIF(b) | ImageKind::WEBP(b) => self.links.get(&key(b)),
            ImageKind::SVG(_) => None,
        }
    }

    /// Did an SVG image have text (which isn't imported)?
    pub(in crate::import) fn nested_text(&self) -> bool {
        self.text.load(Ordering::Relaxed)
    }

    /// The warnings of the SVG images left out.
    pub(in crate::import) fn left_out(&self) -> Vec<String> {
        let mut notes = vec![];
        if self.too_deep.load(Ordering::Relaxed) {
            notes.push(format!("SVG images were left out: with the SVGs around them, their elements nest more than {MAX_ELEMENT_DEPTH} deep"));
        }
        if self.too_big.load(Ordering::Relaxed) {
            notes.push(format!("SVGZ images were left out: together they unpack to more than {} MB", MAX_SVGZ >> 20));
        }
        if self.too_many_svg_images.load(Ordering::Relaxed) {
            notes.push(format!("SVG images were left out: more than {MAX_NESTED_SVG_IMAGES} nested SVG images"));
        }
        notes
    }

    fn note_text(&self, svg: &str) {
        if svg.contains("<text") {
            self.text.store(true, Ordering::Relaxed);
        }
    }

    /// usvg's image resolver, for an SVG whose elements nest `height` deep: data URLs as usvg
    /// reads them, other hrefs only from the files read. SVG images are read here
    /// ([`Files::svg_data`], [`Files::svg`]), never by usvg's own resolver, which caps neither how
    /// deep they nest nor what they unpack to.
    pub(in crate::import) fn resolver(&self, height: usize) -> ImageHrefResolver<'_> {
        self.depth.store(height, Ordering::Relaxed);
        ImageHrefResolver {
            // By type as usvg reads them; one without a type (`text/plain`) by its bytes: a
            // raster usvg reads, else SVG.
            resolve_data: Box::new(move |mime, bytes, opts| {
                let kind = match mime {
                    "image/jpg" | "image/jpeg" => ImageKind::JPEG,
                    "image/png" => ImageKind::PNG,
                    "image/gif" => ImageKind::GIF,
                    "image/webp" => ImageKind::WEBP,
                    "image/svg+xml" => return self.svg_data(&bytes, opts),
                    "text/plain" => match image::guess_format(&bytes).ok().and_then(raster) {
                        Some(kind) => kind,
                        None => return self.svg_data(&bytes, opts),
                    },
                    _ => return None,
                };
                Some(kind(bytes))
            }),
            resolve_string: Box::new(move |h, opts| match self.sources.get(h).or_else(|| self.sources.get(h.trim()))? {
                Source::Raster(kind, bytes) => Some(kind(bytes.clone())),
                Source::Svg(text) => self.svg(text, opts),
            }),
        }
    }

    /// The SVG image of data `bytes` (SVG or SVGZ), as [`Files::svg`] reads it. Unpacked here,
    /// within what the SVGZ images have left, so usvg unpacks nothing itself; past it, `None`,
    /// noted.
    fn svg_data(&self, bytes: &[u8], opts: &usvg::Options) -> Option<ImageKind> {
        match crate::text_within(bytes, &self.unpacked) {
            Ok(text) => self.svg(&text, opts),
            Err(e) => leave_out(&self.too_big, matches!(e, SvgError::TooBig(_))),
        }
    }

    /// The SVG image `text` as usvg reads it with `opts`. `None`, the image left out as one usvg
    /// can't read, when it isn't SVG or when its elements nest deeper than the SVGs around it
    /// leave: usvg reads it while it reads them, on the same stack.
    fn svg(&self, text: &str, opts: &usvg::Options) -> Option<ImageKind> {
        if self.svg_images.fetch_add(1, Ordering::Relaxed) >= MAX_NESTED_SVG_IMAGES {
            return leave_out(&self.too_many_svg_images, true);
        }
        // Measured as the SVG holding it is (`import`): the text before the XML reader.
        let used = self.depth.load(Ordering::Relaxed);
        let left = MAX_ELEMENT_DEPTH.saturating_sub(used);
        if depth::xml_deeper_than(text, left) {
            return leave_out(&self.too_deep, true);
        }
        let height = depth::height(&super::parse(text).ok()?, left);
        let Some(height) = height else { return leave_out(&self.too_deep, true) };
        self.note_text(text);
        self.depth.store(used + height, Ordering::Relaxed);
        let tree = usvg::Tree::from_data_nested(text.as_bytes(), opts);
        self.depth.store(used, Ordering::Relaxed);
        tree.ok().map(ImageKind::SVG)
    }
}

/// How usvg holds a raster of `format`, for the formats it reads.
fn raster(format: image::ImageFormat) -> Option<RasterKind> {
    use image::ImageFormat as F;
    match format {
        F::Png => Some(ImageKind::PNG),
        F::Jpeg => Some(ImageKind::JPEG),
        F::Gif => Some(ImageKind::GIF),
        F::WebP => Some(ImageKind::WEBP),
        _ => None,
    }
}

/// No image: left out, noted in `why` when `noted`.
fn leave_out(why: &AtomicBool, noted: bool) -> Option<ImageKind> {
    if noted {
        why.store(true, Ordering::Relaxed);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decoding_needs_two_hex_digits() {
        assert_eq!(percent_decoded("a%41b"), "aAb");
        assert_eq!(percent_decoded("a%+1b"), "a%+1b");
        assert_eq!(percent_decoded("a%-1b"), "a%-1b");
    }

    #[test]
    fn hrefs_resolve_against_the_folder() {
        let sep = std::path::MAIN_SEPARATOR;
        let art = std::env::temp_dir().join("art");
        let art = art.to_str().unwrap();
        let found = format!("{art}{sep}photo one.png");
        assert_eq!(target("photo%20one.png", Some(art)), (found.clone(), Some(found)));
        assert!(target("photo.png", None).1.is_none());
        assert_eq!(target("file:///C:/art/a.png", None), (r"C:\art\a.png".into(), cfg!(windows).then(|| r"C:\art\a.png".into())));
        assert_eq!(target("https://example.com/a.png", Some(art)), ("https://example.com/a.png".into(), None));
        #[cfg(unix)]
        {
            assert_eq!(target("/abs/photo.png", Some("/art")), ("/abs/photo.png".into(), Some("/abs/photo.png".into())));
            assert_eq!(target("file:///home/a.png", None), ("/home/a.png".into(), Some("/home/a.png".into())));
        }
    }

    /// Without a folder, a link keeps its path as the href spells it, on every system; with one, a
    /// drive path keeps it too. The file to read is the absolute path that `craft_docpath`
    /// resolves on the running system.
    #[test]
    fn links_keep_hrefs_as_written() {
        let art = std::env::temp_dir().join("art");
        let art = art.to_str();
        for h in ["/abs/photo.png", r"\abs\photo.png", "C:/art/a.png", r"C:\art\a.png", "img/a.png"] {
            assert_eq!(target(h, None).0, h);
        }
        for h in ["C:/art/a.png", r"C:\art\a.png"] {
            assert_eq!(target(h, art).0, h);
        }
        assert_eq!(target("/abs/photo%20one.png", None).0, "/abs/photo one.png", "decoded");
        // The SVG's folder gives a rooted path its drive on Windows, and the link keeps that.
        let (link, file) = target("/abs/photo.png", art);
        let file = file.unwrap();
        assert!(Path::new(&file).is_absolute() && Path::new(&file).ends_with("abs/photo.png"), "{file}");
        assert_eq!(link, if cfg!(windows) { file } else { "/abs/photo.png".into() });
    }

    /// A folder that isn't absolute (`/tmp` on Windows, `art`) is taken from the working
    /// directory, so the files its SVG links to are read.
    #[test]
    fn a_folder_that_is_not_absolute_is_made_absolute() {
        let here = std::env::current_dir().unwrap();
        let (link, file) = target("photo.png", Some("art"));
        assert_eq!(file.as_deref().map(Path::new), Some(here.join("art").join("photo.png").as_path()));
        assert_eq!(link, file.unwrap());
        let (_, file) = target("photo.png", Some("/tmp"));
        let file = file.unwrap();
        assert!(Path::new(&file).is_absolute() && file.ends_with("photo.png"), "{file}");
    }

    #[test]
    fn hrefs_to_other_computers_are_not_read() {
        let art = std::env::temp_dir().join("art");
        for h in [
            r"\\192.0.2.1\share\x.png",
            "//192.0.2.1/share/x.png",
            "file:////192.0.2.1/share/x.png",
            "file://192.0.2.1/share/x.png",
            "%5C%5C192.0.2.1%5Cshare%5Cx.png",
        ] {
            assert!(target(h, art.to_str()).1.is_none(), "{h}");
            assert!(target(h, None).1.is_none(), "{h}");
        }
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decoded("a%20b%2"), "a b%2");
        assert_eq!(percent_decoded("%E2%82%AC"), "€");
    }

    /// SVGZ images unpack, all together, to at most what one SVGZ file may: past it, an image is
    /// left out and noted, where usvg unpacked it without a cap (#3563). Text that isn't packed
    /// isn't counted, and a broken SVGZ image is left out as one usvg can't read.
    #[test]
    fn svgz_images_share_the_unpacking_cap() {
        let inner = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="1" height="1"/></svg>"#;
        let (gz, plain) = (Arc::new(crate::compress(inner)), Arc::new(inner.as_bytes().to_vec()));
        let files = Files::default();
        let resolver = files.resolver(2);
        let opts = usvg::Options::default();
        let read = |mime: &str, bytes: &Arc<Vec<u8>>| matches!((resolver.resolve_data)(mime, bytes.clone(), &opts), Some(ImageKind::SVG(_)));
        assert!(read("image/svg+xml", &gz) && read("text/plain", &gz) && read("image/svg+xml", &plain));
        assert_eq!(files.unpacked.load(Ordering::Relaxed), 2 * inner.len() as u64);
        assert!(!read("image/svg+xml", &Arc::new(vec![0x1f, 0x8b, 8, 0, 1, 2])));
        assert!(files.left_out().is_empty(), "broken, not too big");
        // What they unpack to is counted, not the text it decodes to (UTF-16 is twice as long),
        // and counted when it doesn't decode.
        let counted = |bytes: &[u8], svg: bool| {
            let before = files.unpacked.load(Ordering::Relaxed);
            assert_eq!(read("image/svg+xml", &Arc::new(crate::compress_bytes(bytes))), svg);
            files.unpacked.load(Ordering::Relaxed) - before
        };
        let utf16: Vec<u8> = [0xff, 0xfe].into_iter().chain(inner.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(counted(&utf16, true), utf16.len() as u64);
        assert_eq!(counted(&[0xff; 100], false), 100);
        files.unpacked.store(MAX_SVGZ - inner.len() as u64 + 1, Ordering::Relaxed);
        assert!(!read("image/svg+xml", &gz) && !read("text/plain", &gz));
        assert!(read("image/svg+xml", &plain), "not packed");
        assert_eq!(files.left_out(), [format!("SVGZ images were left out: together they unpack to more than {} MB", MAX_SVGZ >> 20)]);
    }

    /// A linked SVGZ file is unpacked once, when it is read, within the same cap.
    #[test]
    fn a_linked_svgz_file_unpacks_once() {
        let inner = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="1" height="1"/></svg>"#;
        let mut files = Files::default();
        let source = files.source(crate::compress(inner), LinkInfo::new("a.svgz")).unwrap();
        files.sources.insert("a.svgz".into(), source);
        assert_eq!(files.unpacked.load(Ordering::Relaxed), inner.len() as u64);
        let resolver = files.resolver(2);
        assert!(matches!((resolver.resolve_string)("a.svgz", &usvg::Options::default()), Some(ImageKind::SVG(_))));
        assert_eq!(files.unpacked.load(Ordering::Relaxed), inner.len() as u64);
        drop(resolver);
        files.unpacked.store(MAX_SVGZ, Ordering::Relaxed);
        assert!(files.source(crate::compress(inner), LinkInfo::new("b.svgz")).is_none(), "past the cap");
    }

    /// An SVG image gets what the SVGs around it leave of the depth cap.
    #[test]
    fn svg_images_get_what_is_left_of_the_depth_cap() {
        let svg = |n: usize| format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{}{}</svg>"#, "<g>".repeat(n), "</g>".repeat(n));
        let read = |around: usize, n: usize| {
            let files = Files::default();
            let resolver = files.resolver(around);
            let kind = (resolver.resolve_data)("image/svg+xml", Arc::new(svg(n).into_bytes()), &usvg::Options::default());
            (matches!(kind, Some(ImageKind::SVG(_))), files.left_out().len())
        };
        assert_eq!(read(1, MAX_ELEMENT_DEPTH - 2), (true, 0));
        assert_eq!(read(2, MAX_ELEMENT_DEPTH - 2), (false, 1));
        assert_eq!(read(MAX_ELEMENT_DEPTH, 0), (false, 1));
    }

    #[test]
    fn the_placeholder_is_a_png() {
        let png = marker().unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), MARKER);
    }
}
