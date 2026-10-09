//! Files `<image>` elements link to (`href="photo.png"`, `file:///…`): read before usvg runs,
//! through [`ImportOptions::read`], relative paths from [`ImportOptions::folder`]. A raster file
//! stays a linked image ([`LinkInfo`]); an SVG file becomes vector art. One that can't be read
//! (missing, remote, not an image) becomes a placeholder that keeps the link and its place, so it
//! can be relinked, and a warning.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use usvg::{ImageHrefResolver, ImageKind, roxmltree};
use vectorcraft_doc::LinkInfo;

use super::{Edits, href};
use crate::ImportOptions;

/// The files an SVG's images link to.
#[derive(Default)]
pub(in crate::import) struct Files {
    /// What usvg gets for each linked href.
    sources: HashMap<String, Source>,
    /// The link of each linked raster or placeholder, by the address of the bytes usvg got.
    links: HashMap<usize, Linked>,
    /// An SVG image had text (which isn't imported).
    text: AtomicBool,
}

enum Source {
    /// A raster file of the kind usvg reads it as.
    Raster(fn(Arc<Vec<u8>>) -> ImageKind, Arc<Vec<u8>>),
    /// An SVG file, parsed by usvg as an image.
    Svg(Arc<Vec<u8>>),
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
        let hex = tail.get(..2).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
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

/// Is `p` an absolute path here or on another system (`/…`, `C:/…`, `C:\…`)?
fn is_absolute(p: &str) -> bool {
    Path::new(p).is_absolute() || p.starts_with('/') || matches!(p.as_bytes(), [d, b':', b'/' | b'\\', ..] if d.is_ascii_alphabetic())
}

/// Where href `h` points (a path, made absolute from `folder`, or a URL) and whether it is a
/// file that can be read.
fn target(h: &str, folder: Option<&str>) -> (String, bool) {
    let h = percent_decoded(h.trim());
    let path = match h.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.strip_prefix("localhost").unwrap_or(rest);
            // `file:///C:/art.png` → `C:/art.png`.
            match rest.as_bytes() {
                [b'/', d, b':', ..] if d.is_ascii_alphabetic() => rest.get(1..).unwrap_or(rest).to_string(),
                _ => rest.to_string(),
            }
        }
        None if h.contains("://") => return (h, false),
        None => h,
    };
    if is_absolute(&path) {
        return (path, true);
    }
    match folder {
        Some(f) => (Path::new(f).join(&path).to_string_lossy().into_owned(), true),
        None => (path, false),
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
                let (path, local) = target(h, opts.folder);
                let read = if local { opts.read.and_then(|read| read(&path)) } else { None };
                let source = match read {
                    Some((bytes, link)) => files.source(bytes, link).ok_or("is not an image Vector W3K2 reads"),
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
        use image::ImageFormat as F;
        let Ok(format) = image::guess_format(&bytes) else {
            let svg = crate::text_of(&bytes).is_ok_and(|t| t.contains("<svg"));
            return svg.then(|| Source::Svg(Arc::new(bytes)));
        };
        let kind = match format {
            F::Png => ImageKind::PNG,
            F::Jpeg => ImageKind::JPEG,
            F::Gif => ImageKind::GIF,
            F::WebP => ImageKind::WEBP,
            _ => {
                let img = image::load_from_memory_with_format(&bytes, format).ok()?;
                let mut png = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut png), F::Png).ok()?;
                let png = Arc::new(png);
                self.links.insert(key(&png), Linked { link, missing: false });
                return Some(Source::Raster(ImageKind::PNG, png));
            }
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

    fn note_text(&self, svg: &[u8]) {
        if svg.windows(5).any(|w| w == b"<text") {
            self.text.store(true, Ordering::Relaxed);
        }
    }

    /// usvg's image resolver: data URLs as usvg reads them, other hrefs only from the files read.
    pub(in crate::import) fn resolver(&self) -> ImageHrefResolver<'_> {
        let data = ImageHrefResolver::default_data_resolver();
        ImageHrefResolver {
            resolve_data: Box::new(move |mime, bytes, opts| {
                if mime == "image/svg+xml" {
                    self.note_text(&bytes);
                }
                data(mime, bytes, opts)
            }),
            resolve_string: Box::new(move |h, opts| match self.sources.get(h).or_else(|| self.sources.get(h.trim()))? {
                Source::Raster(kind, bytes) => Some(kind(bytes.clone())),
                Source::Svg(bytes) => {
                    self.note_text(bytes);
                    usvg::Tree::from_data_nested(bytes, opts).ok().map(ImageKind::SVG)
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hrefs_resolve_against_the_folder() {
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(target("photo%20one.png", Some("/art")), (format!("/art{sep}photo one.png"), true));
        assert_eq!(target("photo.png", None), ("photo.png".into(), false));
        assert_eq!(target("/abs/photo.png", Some("/art")), ("/abs/photo.png".into(), true));
        assert_eq!(target("file:///C:/art/a.png", None), ("C:/art/a.png".into(), true));
        assert_eq!(target("file:///home/a.png", None), ("/home/a.png".into(), true));
        assert_eq!(target("https://example.com/a.png", Some("/art")), ("https://example.com/a.png".into(), false));
    }

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decoded("a%20b%2"), "a b%2");
        assert_eq!(percent_decoded("%E2%82%AC"), "€");
    }

    #[test]
    fn the_placeholder_is_a_png() {
        let png = marker().unwrap();
        assert_eq!(image::load_from_memory(&png).unwrap().width(), MARKER);
    }
}
