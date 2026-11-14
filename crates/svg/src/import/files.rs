//! Files `<image>` elements link to (`href="photo.png"`, `file:///…`): read before usvg runs,
//! through [`ImportOptions::read`], relative paths from [`ImportOptions::folder`]. A raster file
//! stays a linked image ([`LinkInfo`]); an SVG file becomes vector art. One that can't be read
//! (missing, remote, not an image) becomes a placeholder that keeps the link and its place, so it
//! can be relinked, and a warning.
//!
//! A linked SVG file's own `<image>` links are read the same way, relative paths from that file's
//! folder, within the same limits; an href that leads back to an SVG file showing it gets the
//! placeholder and a warning.
//!
//! A raster image, linked or a data URL, whose orientation (EXIF, or a TIFF's Orientation tag)
//! turns or flips its pixels is turned before usvg gets it, so it is sized, fitted and stored as
//! it is shown.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use usvg::{ImageHrefResolver, ImageKind, roxmltree};
use vectorcraft_doc::LinkInfo;
use vectorcraft_doc::links::missing_placeholder;

use super::{Edits, depth, href};
use crate::{ImportOptions, MAX_ELEMENT_DEPTH, MAX_SVGZ, SvgError};

/// Most nested SVG images an imported SVG may decode (#3564).
pub const MAX_NESTED_SVG_IMAGES: usize = 256;

/// Most nested SVG images an imported SVG may read, decoded or not: each one read is checked and
/// parsed, and an image that fails is read again wherever it is shown.
pub const MAX_NESTED_SVG_READS: usize = 1024;

/// Most bytes of nested SVG image text an imported SVG may read, decoded or not, all reads together.
pub const MAX_NESTED_SVG_TEXT: u64 = 512 << 20;

/// The files an SVG's images link to.
#[derive(Default)]
pub(in crate::import) struct Files {
    /// What usvg gets for each linked href.
    sources: HashMap<String, Source>,
    /// The SVG images usvg is reading, innermost last: a linked SVG file, whose own links an href
    /// inside it is looked up in, or `None` for a data URL.
    within: Mutex<Vec<Option<Arc<SvgFile>>>>,
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
    /// How many nested SVG images were read so far, decoded or not.
    svg_reads: AtomicUsize,
    /// The bytes of text those reads took.
    svg_text: AtomicU64,
    /// SVG images were left out for exceeding [`MAX_NESTED_SVG_READS`] or [`MAX_NESTED_SVG_TEXT`].
    too_much_svg_read: AtomicBool,
    /// The CMYK TIFF stored in place of a turned CMYK image, by the address of the PNG usvg got
    /// (kept here too, so the address stays its own).
    cmyk: Mutex<HashMap<usize, (Bytes, Bytes)>>,
}

/// Encoded image bytes, shared.
type Bytes = Arc<Vec<u8>>;

/// Raster bytes as usvg gets them.
struct Shown {
    kind: RasterKind,
    bytes: Arc<Vec<u8>>,
    /// The CMYK TIFF stored in their place: a turned CMYK image's inks, turned.
    cmyk: Option<Vec<u8>>,
}

/// How usvg holds the bytes of a raster of one format (`ImageKind::PNG`…).
type RasterKind = fn(Arc<Vec<u8>>) -> ImageKind;

#[derive(Clone)]
enum Source {
    /// A raster file of the kind usvg reads it as.
    Raster(RasterKind, Arc<Vec<u8>>),
    /// An SVG file, parsed by usvg as an image.
    Svg(Arc<SvgFile>),
}

/// A linked SVG file as usvg reads it.
struct SvgFile {
    /// Its text (unpacked when it is SVGZ), placeholders stretched to their boxes.
    text: Arc<str>,
    /// What usvg gets for each href its own `<image>` elements link to.
    sources: HashMap<String, Source>,
}

/// The state of [`Files::read`] across the SVG files it reads the links of.
struct Reading<'a, 'w> {
    opts: &'a ImportOptions<'a>,
    warnings: &'w mut Vec<String>,
    /// The placeholder's bytes, made once.
    placeholder: Option<Option<Vec<u8>>>,
    /// What each file read gave, by the file: hrefs spelled differently (`a.png`, `./a.png`,
    /// `%61.png`), and in different SVG files, share one read and one copy of its bytes. An SVG
    /// file with a link inside it that led back to an SVG file isn't kept: what it shows depends
    /// on where it is.
    by_file: HashMap<String, Result<Source, &'static str>>,
    /// The SVG files whose links are being read, innermost last, by [`identity`].
    active: Vec<String>,
    /// A link led back to one of them while the file being read was read.
    led_back: bool,
    /// How many linked SVG files were read for their links, and the bytes of their text.
    svg_reads: usize,
    svg_text: u64,
}

/// What the link to an SVG file being read is compared by, to find a link back to it: its path
/// with links and `..` resolved, or the path as given when that can't be found.
fn identity(file: &str) -> String {
    std::fs::canonicalize(file).map_or_else(|_| file.to_string(), |p| p.to_string_lossy().into_owned())
}

/// A linked raster image (or the placeholder of a missing one).
pub(in crate::import) struct Linked {
    pub link: LinkInfo,
    /// The file wasn't read: the bytes are a placeholder.
    pub missing: bool,
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
        let mut reading =
            Reading { opts, warnings, placeholder: None, by_file: HashMap::new(), active: vec![], led_back: false, svg_reads: 0, svg_text: 0 };
        files.sources = files.links_of(svg, xml, opts.folder, None, edits, &mut reading);
        files
    }

    /// What usvg gets for each href the `<image>` elements of `xml` (the text `svg`) link to,
    /// relative paths taken from `folder`. `inside` is how deep the SVG images `xml` shows nest in
    /// it and the SVGs around it, measured when an SVG file is read (`None` for `xml` itself).
    fn links_of(
        &mut self,
        svg: &str,
        xml: &roxmltree::Document,
        folder: Option<&str>,
        mut inside: Option<usize>,
        edits: &mut Edits,
        reading: &mut Reading<'_, '_>,
    ) -> HashMap<String, Source> {
        let mut sources: HashMap<String, Source> = HashMap::new();
        for n in xml.descendants().filter(|n| n.is_element() && n.tag_name().name() == "image") {
            let Some(h) = href(n).filter(|h| !h.trim_start().starts_with("data:")) else { continue };
            if !sources.contains_key(h) {
                let (path, file) = target(h, folder);
                // A failed read is cached too, but each href gets its own placeholder: its own link
                // path (an Arc shared between hrefs would share the first href's link) and warning.
                let back = !reading.active.is_empty() && file.as_deref().is_some_and(|f| reading.active.contains(&identity(f)));
                let cached = file.as_ref().and_then(|f| reading.by_file.get(f)).cloned();
                let source = match cached {
                    _ if back => {
                        reading.led_back = true;
                        Err("leads back to an SVG file that shows it")
                    }
                    Some(source) => source,
                    None => {
                        let read = file.clone().and_then(|file| reading.opts.read.and_then(|read| read(&file)));
                        let around = std::mem::replace(&mut reading.led_back, false);
                        let source = match (read, file.as_deref()) {
                            (Some((bytes, link)), Some(f)) => match self.source(bytes, link) {
                                Some(Source::Svg(text)) => {
                                    let inside = *inside.get_or_insert_with(|| depth::height(xml, MAX_ELEMENT_DEPTH).unwrap_or(MAX_ELEMENT_DEPTH));
                                    // Past what may be read of the SVG files, the image is left out (noted).
                                    let Some(source) = self.svg_file(text, f, inside, reading) else {
                                        reading.led_back = around;
                                        continue;
                                    };
                                    Ok(source)
                                }
                                source => source.ok_or("is not an image VectorCraft reads"),
                            },
                            _ => Err("not found"),
                        };
                        // Kept unless a link inside it led back to an SVG file.
                        let led_back = reading.led_back;
                        reading.led_back |= around;
                        if let Some(file) = file
                            && !led_back
                        {
                            reading.by_file.insert(file, source.clone());
                        }
                        source
                    }
                };
                let source = source.or_else(|why| {
                    reading.warnings.push(format!("linked image '{h}' {why}: a placeholder keeps its place until it is relinked"));
                    let bytes = Arc::new(reading.placeholder.get_or_insert_with(missing_placeholder).clone().ok_or(why)?);
                    self.links.insert(key(&bytes), Linked { link: LinkInfo::new(path), missing: true });
                    Ok::<_, &str>(Source::Raster(ImageKind::PNG, bytes))
                });
                let Ok(source) = source else { continue };
                sources.insert(h.to_string(), source);
            }
            if sources.get(h).is_some_and(|s| self.missing(s)) {
                edits.set(svg, n, "preserveAspectRatio", "none");
            }
        }
        sources
    }

    /// Linked SVG file `file`, read as `svg`, with the files its own `<image>` elements link to
    /// read from its folder; `inside` is how deep it nests in the SVGs around it. Its links aren't
    /// read when it nests too deep to be shown, or when it isn't XML. `None`, noted, once the SVG
    /// files read for their links pass [`MAX_NESTED_SVG_READS`] or [`MAX_NESTED_SVG_TEXT`].
    fn svg_file(&mut self, svg: Arc<SvgFile>, file: &str, inside: usize, reading: &mut Reading<'_, '_>) -> Option<Source> {
        let left = MAX_ELEMENT_DEPTH.saturating_sub(inside);
        if !svg.text.contains("image") || depth::xml_deeper_than(&svg.text, left) {
            return Some(Source::Svg(svg));
        }
        reading.svg_reads += 1;
        reading.svg_text = reading.svg_text.saturating_add(svg.text.len() as u64);
        if reading.svg_reads > MAX_NESTED_SVG_READS || reading.svg_text > MAX_NESTED_SVG_TEXT {
            self.too_much_svg_read.store(true, Ordering::Relaxed);
            return None;
        }
        // Read from its own handle: the text stays borrowed while `svg` may be given back.
        let text = svg.text.clone();
        let Ok(xml) = super::parse(&text) else { return Some(Source::Svg(svg)) };
        let Some(height) = depth::height(&xml, left) else { return Some(Source::Svg(svg)) };
        let folder = Path::new(file).parent().map(|p| p.to_string_lossy().into_owned());
        let mut edits = Edits::default();
        reading.active.push(identity(file));
        let sources = self.links_of(&text, &xml, folder.as_deref(), Some(inside + height), &mut edits, reading);
        reading.active.pop();
        let text = Arc::from(edits.apply(&text).as_ref());
        Some(Source::Svg(Arc::new(SvgFile { text, sources })))
    }

    /// What usvg gets for a file read (`bytes`, linked by `link`): a raster as [`shown`] gives it
    /// (formats usvg doesn't read are stored as PNG) or an SVG, its own links not read yet; `None`
    /// for anything else.
    fn source(&mut self, bytes: Vec<u8>, link: LinkInfo) -> Option<Source> {
        let Ok(format) = image::guess_format(&bytes) else {
            // Unpacked once, here, within what the SVGZ images may unpack to.
            let text = crate::text_within(&bytes, &self.unpacked).ok()?;
            return text.contains("<svg").then(|| Source::Svg(Arc::new(SvgFile { text: Arc::from(text.as_ref()), sources: HashMap::new() })));
        };
        let (kind, bytes) = self.keep(shown(Arc::new(bytes), format)?);
        self.links.insert(key(&bytes), Linked { link, missing: false });
        Some(Source::Raster(kind, bytes))
    }

    /// What usvg gets for `shown`, with its CMYK TIFF, when it has one, noted to store in its
    /// place ([`Files::stored`]).
    fn keep(&self, shown: Shown) -> (RasterKind, Arc<Vec<u8>>) {
        if let Some(cmyk) = shown.cmyk
            && let Ok(mut stored) = self.cmyk.lock()
        {
            stored.insert(key(&shown.bytes), (shown.bytes.clone(), Arc::new(cmyk)));
        }
        (shown.kind, shown.bytes)
    }

    /// The CMYK TIFF stored in place of the image whose data usvg got as `kind`: a CMYK JPEG or
    /// TIFF its orientation turns, which usvg got as PNG.
    pub(in crate::import) fn stored(&self, kind: &ImageKind) -> Option<Arc<Vec<u8>>> {
        let ImageKind::PNG(b) = kind else { return None };
        self.cmyk.lock().ok()?.get(&key(b)).map(|(_, tiff)| tiff.clone())
    }

    /// Is `source` a placeholder?
    fn missing(&self, source: &Source) -> bool {
        matches!(source, Source::Raster(_, b) if self.links.get(&key(b)).is_some_and(|l| l.missing))
    }

    /// What usvg gets for href `h` where it is read: inside a linked SVG file, from that file's
    /// own links; outside any SVG image, from the imported SVG's. Inside an SVG data URL, which
    /// has no folder of its own, nothing.
    fn find(&self, h: &str) -> Option<Source> {
        let within = self.within.lock().ok()?;
        let sources = match within.last() {
            None => &self.sources,
            Some(file) => &file.as_ref()?.sources,
        };
        sources.get(h).or_else(|| sources.get(h.trim())).cloned()
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
        if self.too_much_svg_read.load(Ordering::Relaxed) {
            notes.push(format!(
                "SVG images were left out: nested SVG images, valid or not, are read at most {MAX_NESTED_SVG_READS} times and {} MB of text",
                MAX_NESTED_SVG_TEXT >> 20
            ));
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
            // By type as usvg reads them, and TIFF; one without a type (`text/plain`) by its
            // bytes: one of those rasters, else SVG. A raster as [`shown`] gives it.
            resolve_data: Box::new(move |mime, bytes, opts| {
                use image::ImageFormat as F;
                let format = match mime {
                    "image/jpg" | "image/jpeg" => F::Jpeg,
                    "image/png" => F::Png,
                    "image/gif" => F::Gif,
                    "image/webp" => F::WebP,
                    "image/tiff" => F::Tiff,
                    "image/svg+xml" => return self.svg_data(&bytes, opts),
                    "text/plain" => match image::guess_format(&bytes).ok().filter(|f| raster(*f).is_some() || *f == F::Tiff) {
                        Some(format) => format,
                        None => return self.svg_data(&bytes, opts),
                    },
                    _ => return None,
                };
                let (kind, bytes) = self.keep(shown(bytes, format)?);
                Some(kind(bytes))
            }),
            resolve_string: Box::new(move |h, opts| match self.find(h)? {
                Source::Raster(kind, bytes) => Some(kind(bytes)),
                Source::Svg(file) => self.svg(&file.text.clone(), Some(file), opts),
            }),
        }
    }

    /// The SVG image of data `bytes` (SVG or SVGZ), as [`Files::svg`] reads it. Unpacked here,
    /// within what the SVGZ images have left, so usvg unpacks nothing itself; past it, `None`,
    /// noted.
    fn svg_data(&self, bytes: &[u8], opts: &usvg::Options) -> Option<ImageKind> {
        match crate::text_within(bytes, &self.unpacked) {
            Ok(text) => self.svg(&text, None, opts),
            Err(e) => leave_out(&self.too_big, matches!(e, SvgError::TooBig(_))),
        }
    }

    /// The SVG image `text` as usvg reads it with `opts`. `None`, the image left out as one usvg
    /// can't read, when it isn't SVG or when its elements nest deeper than the SVGs around it
    /// leave: usvg reads it while it reads them, on the same stack. Only a decoded image, with the
    /// images decoded inside it, counts toward [`MAX_NESTED_SVG_IMAGES`]; every image read counts
    /// toward [`MAX_NESTED_SVG_READS`] and [`MAX_NESTED_SVG_TEXT`]. `file` is the linked SVG file
    /// the text is, whose own links the hrefs in it are looked up in.
    fn svg(&self, text: &str, file: Option<Arc<SvgFile>>, opts: &usvg::Options) -> Option<ImageKind> {
        let decoded = self.svg_images.load(Ordering::Relaxed);
        if decoded >= MAX_NESTED_SVG_IMAGES {
            return leave_out(&self.too_many_svg_images, true);
        }
        let reads = self.svg_reads.load(Ordering::Relaxed);
        let read = self.svg_text.load(Ordering::Relaxed).saturating_add(text.len() as u64);
        if reads >= MAX_NESTED_SVG_READS || read > MAX_NESTED_SVG_TEXT {
            return leave_out(&self.too_much_svg_read, true);
        }
        self.svg_reads.store(reads + 1, Ordering::Relaxed);
        self.svg_text.store(read, Ordering::Relaxed);
        // The image's place is taken while usvg reads it, so the images inside it get what is left.
        // usvg reads them all on this thread, so no other read comes between these steps.
        self.svg_images.store(decoded + 1, Ordering::Relaxed);
        let tree = self.svg_tree(text, file, opts);
        if tree.is_none() {
            self.svg_images.store(decoded, Ordering::Relaxed);
        }
        tree
    }

    /// [`Files::svg`]'s image once it is counted.
    fn svg_tree(&self, text: &str, file: Option<Arc<SvgFile>>, opts: &usvg::Options) -> Option<ImageKind> {
        // Measured as the SVG holding it is (`import`): the text before the XML reader.
        let used = self.depth.load(Ordering::Relaxed);
        let left = MAX_ELEMENT_DEPTH.saturating_sub(used);
        if depth::xml_deeper_than(text, left) {
            return leave_out(&self.too_deep, true);
        }
        let xml = super::parse(text).ok()?;
        let Some(height) = depth::height(&xml, left) else { return leave_out(&self.too_deep, true) };
        // usvg's parser would overflow on it, as on the SVG holding it (`import`).
        super::convolve_matrices_ok(&xml).ok()?;
        self.depth.store(used + height, Ordering::Relaxed);
        let within = self.within.lock().ok().map(|mut w| w.push(file)).is_some();
        // usvg's `Tree::from_data_nested`, except that hrefs to files go to the resolver.
        let nested = usvg::Options {
            resources_dir: None,
            dpi: opts.dpi,
            font_size: opts.font_size,
            languages: opts.languages.clone(),
            shape_rendering: opts.shape_rendering,
            text_rendering: opts.text_rendering,
            image_rendering: opts.image_rendering,
            default_size: opts.default_size,
            image_href_resolver: ImageHrefResolver {
                resolve_data: Box::new(|mime, bytes, o| (opts.image_href_resolver.resolve_data)(mime, bytes, o)),
                resolve_string: Box::new(|h, o| (opts.image_href_resolver.resolve_string)(h, o)),
            },
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(text, &nested);
        if within && let Ok(mut w) = self.within.lock() {
            w.pop();
        }
        self.depth.store(used, Ordering::Relaxed);
        let tree = tree.ok()?;
        self.note_text(text);
        Some(ImageKind::SVG(tree))
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

/// Encoded raster `bytes`, `format` by their type (or by their bytes when they have none), as usvg
/// gets them, as Open and Place store them: unchanged when usvg reads the format and the image's
/// orientation ([`vectorcraft_doc::cmyk::exif_orientation`], read for PNG, JPEG, GIF, WebP and
/// TIFF) leaves the pixels as stored. Otherwise the pixels are read ([`decoded`],
/// [`tiff_pixels`]), turned or flipped as the orientation says, and written as PNG, so usvg sizes
/// and fits the image as it is shown. A CMYK TIFF also keeps its inks as a CMYK TIFF
/// ([`vectorcraft_doc::cmyk::stored_cmyk_tiff`]), and a CMYK JPEG or TIFF that is turned keeps them
/// turned ([`vectorcraft_doc::cmyk::turned_cmyk`]). `None` when the pixels can't be read, and for
/// a TIFF with more strips or tiles than the decoder can count
/// ([`vectorcraft_doc::cmyk::oversized_tiff`]), before any decoder opens it.
fn shown(bytes: Arc<Vec<u8>>, format: image::ImageFormat) -> Option<Shown> {
    use image::ImageFormat as F;
    if vectorcraft_doc::cmyk::oversized_tiff(&bytes).is_some() {
        return None;
    }
    let orientation =
        matches!(format, F::Png | F::Jpeg | F::Gif | F::WebP | F::Tiff).then(|| vectorcraft_doc::cmyk::exif_orientation(&bytes)).flatten();
    if let Some(kind) = raster(format)
        && orientation.is_none()
    {
        return Some(Shown { kind, bytes, cmyk: None });
    }
    let tiff = image::guess_format(&bytes).ok() == Some(F::Tiff);
    let mut img = if tiff { tiff_pixels(&bytes)? } else { decoded(&bytes, format)? };
    let cmyk = match orientation {
        Some(o) if matches!(format, F::Jpeg | F::Tiff) => vectorcraft_doc::cmyk::turned_cmyk(&bytes, o, None).map(|(tiff, _)| tiff),
        None if tiff => vectorcraft_doc::cmyk::stored_cmyk_tiff(&bytes, None),
        _ => None,
    };
    if let Some(orientation) = orientation {
        img.apply_orientation(orientation);
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), F::Png).ok()?;
    Some(Shown { kind: ImageKind::PNG, bytes: Arc::new(png), cmyk })
}

/// The pixels of encoded image `bytes` (`format` when their bytes don't say), as stored, decoded
/// within [`vectorcraft_doc::MAX_RASTER_ALLOC`], counted as decoded and as RGBA, as Open and Place
/// decode them; `None` when they can't be.
fn decoded(bytes: &[u8], format: image::ImageFormat) -> Option<image::DynamicImage> {
    use image::ImageDecoder as _;
    let max = vectorcraft_doc::MAX_RASTER_ALLOC;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    if reader.format().is_none() {
        reader.set_format(format);
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(max);
    reader.limits(limits);
    let decoder = reader.into_decoder().ok()?;
    let (w, h) = decoder.dimensions();
    if decoder.total_bytes() > max || u64::from(w) * u64::from(h) * 4 > max {
        return None;
    }
    image::DynamicImage::from_decoder(decoder).ok()
}

/// The pixels of TIFF `bytes`, as stored, read as Open and Place read them: a TIFF with extra
/// samples (CMYK or gray with alpha) as RGBA, CMYK converted with the active color settings
/// ([`vectorcraft_doc::cmyk::extra_samples_tiff_rgba`]); a 1-bit TIFF compressed with CCITT Group
/// 3 as RGBA ([`vectorcraft_doc::cmyk::group3_tiff_rgba`]); others by [`decoded`], a CMYK TIFF
/// that stores one plane per ink with its inks written together first
/// ([`vectorcraft_doc::cmyk::stored_cmyk_tiff`]). `None` when they can't be read.
fn tiff_pixels(bytes: &[u8]) -> Option<image::DynamicImage> {
    use image::DynamicImage::ImageRgba8;
    let cms = vectorcraft_color::cms::active();
    if let Some(img) = vectorcraft_doc::cmyk::extra_samples_tiff_rgba(bytes, |c| cms.cmyk_to_srgb(c, false)) {
        return Some(ImageRgba8(img));
    }
    if let Some(read) = vectorcraft_doc::cmyk::group3_tiff_rgba(bytes) {
        return read.ok().map(ImageRgba8);
    }
    decoded(bytes, image::ImageFormat::Tiff).or_else(|| decoded(&vectorcraft_doc::cmyk::stored_cmyk_tiff(bytes, None)?, image::ImageFormat::Tiff))
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

    /// An SVG image with a `feConvolveMatrix` order usvg's multiply overflows is left out before
    /// usvg reads it, as the SVG holding it would be refused.
    #[test]
    fn svg_images_with_an_overflowing_convolve_order_are_left_out() {
        let read = |order: &str| {
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><filter id="f"><feConvolveMatrix order="{order}" kernelMatrix="1"/></filter><rect width="4" height="4" filter="url(#f)"/></svg>"#
            );
            let files = Files::default();
            let resolver = files.resolver(1);
            matches!((resolver.resolve_data)("image/svg+xml", Arc::new(svg.into_bytes()), &usvg::Options::default()), Some(ImageKind::SVG(_)))
        };
        assert!(!read("2147483647"));
        assert!(read("1"));
    }

    /// The nested SVG images read, decoded or not, take at most [`MAX_NESTED_SVG_TEXT`] bytes of
    /// text together: an image whose text goes past it is left out and noted. A broken image's
    /// text counts too, and its decoded place is given back.
    #[test]
    fn svg_images_share_the_text_cap() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="1" height="1"/></svg>"#;
        let broken = &svg[..20];
        let files = Files::default();
        let resolver = files.resolver(1);
        let opts = usvg::Options::default();
        let read =
            |text: &str| matches!((resolver.resolve_data)("image/svg+xml", Arc::new(text.as_bytes().to_vec()), &opts), Some(ImageKind::SVG(_)));
        assert!(!read(broken));
        assert_eq!(files.svg_text.load(Ordering::Relaxed), broken.len() as u64);
        assert_eq!((files.svg_reads.load(Ordering::Relaxed), files.svg_images.load(Ordering::Relaxed)), (1, 0));
        files.svg_text.store(MAX_NESTED_SVG_TEXT - svg.len() as u64, Ordering::Relaxed);
        assert!(read(svg));
        assert_eq!((files.svg_reads.load(Ordering::Relaxed), files.svg_images.load(Ordering::Relaxed)), (2, 1));
        assert!(files.left_out().is_empty());
        assert!(!read(svg) && !read(broken));
        assert_eq!(files.svg_text.load(Ordering::Relaxed), MAX_NESTED_SVG_TEXT, "past the cap, nothing is read");
        assert_eq!(
            files.left_out(),
            [format!(
                "SVG images were left out: nested SVG images, valid or not, are read at most {MAX_NESTED_SVG_READS} times and {} MB of text",
                MAX_NESTED_SVG_TEXT >> 20
            )]
        );
    }
}
