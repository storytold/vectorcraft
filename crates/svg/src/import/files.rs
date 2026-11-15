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
//!
//! usvg decodes a data URL each time it converts the element holding it, a `<use>` copy
//! included, before the resolver sees it. So before usvg reads an SVG (the imported one, and
//! each SVG image), each `<image>` and `<feImage>` href that is a data URL is changed to a key,
//! and the resolver decodes the URL once, the first time a key comes, within what the data URLs
//! may decode to together ([`MAX_DATA_URL_BYTES`]); every later use of it gets the same image.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use usvg::{ImageHrefResolver, ImageKind, roxmltree};
use vectorcraft_doc::LinkInfo;
use vectorcraft_doc::links::missing_placeholder;

use super::{Edits, budget, depth, href};
use crate::{ImportOptions, MAX_ELEMENT_DEPTH, MAX_SVGZ, SvgError};

/// Most nested SVG images an imported SVG may decode (#3564).
pub const MAX_NESTED_SVG_IMAGES: usize = 256;

/// Most nested SVG images an imported SVG may read, decoded or not: each one read is checked and
/// parsed, and an image that fails is read again wherever it is shown.
pub const MAX_NESTED_SVG_READS: usize = 1024;

/// Most bytes of nested SVG image text an imported SVG may read, decoded or not, all reads together.
pub const MAX_NESTED_SVG_TEXT: u64 = 512 << 20;

/// Most bytes the image data URLs an imported SVG shows may decode to, those in its SVG images
/// included, all together; each URL is decoded once, however often it is shown.
pub const MAX_DATA_URL_BYTES: u64 = 512 << 20;

/// What a data URL's key starts with, the URL's number following it.
const DATA_URL_KEY: &str = "vectorcraft-data-url-";

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
    /// Images linked from linked SVG files were left out, their files not read, past the same
    /// limits for the files read for links.
    too_much_linked: AtomicBool,
    /// The CMYK TIFF stored in place of a turned CMYK image, by the address of the PNG usvg got
    /// (kept here too, so the address stays its own).
    cmyk: Mutex<HashMap<usize, (Bytes, Bytes)>>,
    /// The data URLs changed to keys before usvg read them.
    data: Mutex<DataUrls>,
    /// The bytes the image data URLs decoded to so far, whether or not they are images.
    data_bytes: AtomicU64,
    /// Images were left out for their data URLs decoding past [`MAX_DATA_URL_BYTES`].
    too_much_data: AtomicBool,
    /// What usvg makes of the SVG and of the SVG images read so far, all together ([`budget`]):
    /// elements, bytes of copied style values and selector tests.
    spent: Mutex<budget::Cost>,
    /// Why an SVG image refused the whole SVG: what usvg would make of it is past [`budget`]'s
    /// limits. The first one.
    refused: Mutex<Option<SvgError>>,
}

/// The hrefs changed to keys, by number: an href's key is [`DATA_URL_KEY`] and its number.
#[derive(Default)]
struct DataUrls {
    /// The number of each href.
    numbers: HashMap<Arc<str>, usize>,
    /// Each href: a data URL, then what it decoded to, or a link.
    urls: Vec<DataUrl>,
    /// Each href as it was written, for the filters kept as written ([`Files::data_href`]).
    hrefs: Vec<Arc<str>>,
}

enum DataUrl {
    Encoded(Arc<str>),
    Decoded(Decoded),
    /// A link that starts as a key does, looked up as it is.
    Link(Arc<str>),
}

/// What a data URL decoded to, as usvg gets it.
#[derive(Clone)]
enum Decoded {
    Raster(RasterKind, Bytes),
    /// SVG text (unpacked when it was SVGZ), read where it is shown ([`Files::svg`]).
    Svg(Arc<str>),
    /// Not an image VectorCraft reads, or left out.
    Nothing,
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
    /// `%61.png`), and in different SVG files, share one read and one copy of its bytes. Only an
    /// SVG file whose art depends on where it is shown ([`Reading::placed`]) isn't kept, and a
    /// raster read past the limits, which is left out.
    by_file: HashMap<String, Outcome>,
    /// The SVG files whose links are being read, innermost last, by [`identity`].
    active: Vec<String>,
    /// What the file being read gives depends on where it is shown: a link inside it led back to
    /// an SVG file around it, or an SVG file inside it nests too deep there for its links to be
    /// read.
    placed: bool,
    /// How many linked SVG files were read for their links, and the bytes of every linked SVG
    /// file read and of every file read for the links of a linked SVG file. Rasters the imported
    /// SVG links to aren't counted.
    svg_reads: usize,
    svg_text: u64,
    /// What the linked SVG files measured for their links make, all together ([`budget`]); of one
    /// past what is left, the selector tests it made.
    measured: budget::Cost,
}

impl Reading<'_, '_> {
    /// The files read for links passed [`MAX_NESTED_SVG_READS`] or [`MAX_NESTED_SVG_TEXT`].
    fn spent(&self) -> bool {
        self.svg_reads > MAX_NESTED_SVG_READS || self.svg_text > MAX_NESTED_SVG_TEXT
    }
}

/// What reading a linked file gave: what usvg gets, or why it can't be read.
#[derive(Clone)]
struct Outcome {
    source: Result<Source, &'static str>,
    /// An SVG file whose own links weren't read, past [`Reading::spent`]: inside a linked SVG
    /// file it is left out, and where the imported SVG links to it, it is shown without them.
    capped: bool,
}

impl Outcome {
    fn not_read(why: &'static str) -> Self {
        Outcome { source: Err(why), capped: false }
    }

    fn shown(source: Source) -> Self {
        Outcome { source: Ok(source), capped: false }
    }
}

/// Does SVG text `text` have an `<image>` that links to a file (not a data URL)?
fn links_files(text: &str) -> bool {
    super::parse(text).is_ok_and(|xml| xml.descendants().any(|n| n.is_element() && n.tag_name().name() == "image" && file_href(n).is_some()))
}

/// The href of `<image>` element `n`, unless it is a data URL.
fn file_href<'a>(n: roxmltree::Node<'a, '_>) -> Option<&'a str> {
    href(n).filter(|h| !h.trim_start().starts_with("data:"))
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
    /// relinked. The data URLs of its images are changed to keys ([`Files::take_data_urls`]).
    pub(in crate::import) fn read(svg: &str, xml: &roxmltree::Document, opts: &ImportOptions, edits: &mut Edits, warnings: &mut Vec<String>) -> Self {
        let mut files = Files::default();
        let mut reading = Reading {
            opts,
            warnings,
            placeholder: None,
            by_file: HashMap::new(),
            active: vec![],
            placed: false,
            svg_reads: 0,
            svg_text: 0,
            measured: budget::Cost::default(),
        };
        files.sources = files.links_of(svg, xml, opts.folder, None, edits, &mut reading);
        files.take_data_urls(xml, edits);
        files
    }

    /// Change each href of the `<image>` and `<feImage>` elements of `xml` that usvg would decode
    /// as a data URL to that URL's key, numbering the URLs not seen before: usvg gives a key to
    /// the resolver as it is ([`Files::data_url`]). An href that starts as a key does, a link,
    /// gets a key too, which the resolver reads as that link.
    fn take_data_urls(&self, xml: &roxmltree::Document, edits: &mut Edits) {
        let Ok(mut data) = self.data.lock() else { return };
        let hrefs = xml
            .descendants()
            .filter(|n| n.is_element() && matches!(n.tag_name().name(), "image" | "feImage"))
            .flat_map(|n| n.attributes())
            .filter(|a| a.name() == "href" && matches!(a.namespace(), None | Some("http://www.w3.org/1999/xlink")));
        for a in hrefs {
            let url = data_url::DataUrl::process(a.value()).is_ok();
            if !url && !a.value().starts_with(DATA_URL_KEY) {
                continue;
            }
            let n = match data.numbers.get(a.value()) {
                Some(n) => *n,
                None => {
                    let href: Arc<str> = Arc::from(a.value());
                    let n = data.urls.len();
                    data.numbers.insert(href.clone(), n);
                    data.hrefs.push(href.clone());
                    data.urls.push(if url { DataUrl::Encoded(href) } else { DataUrl::Link(href) });
                    n
                }
            };
            edits.set_value(a, &format!("{DATA_URL_KEY}{n}"));
        }
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
        let nested = !reading.active.is_empty();
        for n in xml.descendants().filter(|n| n.is_element() && n.tag_name().name() == "image") {
            let Some(h) = file_href(n) else { continue };
            if !sources.contains_key(h) {
                let (path, file) = target(h, folder);
                // A failed read is cached too, but each href gets its own placeholder: its own link
                // path (an Arc shared between hrefs would share the first href's link) and warning.
                let back = nested && file.as_deref().is_some_and(|f| reading.active.contains(&identity(f)));
                let cached = file.as_ref().and_then(|f| reading.by_file.get(f)).cloned();
                let outcome = match cached {
                    _ if back => {
                        reading.placed = true;
                        Some(Outcome::not_read("leads back to an SVG file that shows it"))
                    }
                    Some(outcome) => Some(outcome),
                    None => {
                        let around = std::mem::replace(&mut reading.placed, false);
                        let outcome = self.file(file.as_deref(), xml, &mut inside, reading);
                        // Kept unless what it shows depends on where it is.
                        let placed = reading.placed;
                        reading.placed |= around;
                        if let (Some(file), Some(outcome)) = (file, &outcome)
                            && !placed
                        {
                            reading.by_file.insert(file, outcome.clone());
                        }
                        outcome
                    }
                };
                // Inside a linked SVG file, past what may be read, the image is left out (noted).
                let source = match outcome {
                    None => continue,
                    Some(Outcome { capped: true, .. }) if nested => {
                        self.too_much_svg_read.store(true, Ordering::Relaxed);
                        continue;
                    }
                    Some(outcome) => outcome.source,
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

    /// What reading linked file `file` gives, linked from `xml`, whose SVG images nest `inside`
    /// deep (measured here when an SVG file is read); `None` for an image left out, noted. Inside
    /// a linked SVG file each file read counts toward [`MAX_NESTED_SVG_TEXT`] before it is looked
    /// at, and once the files read for links pass [`MAX_NESTED_SVG_READS`] or
    /// [`MAX_NESTED_SVG_TEXT`] no file is read there; a raster whose read passes them is left out
    /// undecoded. Every file the imported SVG links to is read.
    fn file(&mut self, file: Option<&str>, xml: &roxmltree::Document, inside: &mut Option<usize>, reading: &mut Reading<'_, '_>) -> Option<Outcome> {
        let (Some(file), Some(read)) = (file, reading.opts.read) else { return Some(Outcome::not_read("not found")) };
        let nested = !reading.active.is_empty();
        if nested && reading.spent() {
            self.too_much_linked.store(true, Ordering::Relaxed);
            return None;
        }
        let Some((bytes, link)) = read(file) else { return Some(Outcome::not_read("not found")) };
        let counted = if nested { bytes.len() as u64 } else { 0 };
        reading.svg_text = reading.svg_text.saturating_add(counted);
        if nested && reading.spent() && image::guess_format(&bytes).is_ok() {
            self.too_much_linked.store(true, Ordering::Relaxed);
            return None;
        }
        match self.source(bytes, link) {
            Some(Source::Svg(svg)) => {
                let inside = *inside.get_or_insert_with(|| depth::height(xml, MAX_ELEMENT_DEPTH).unwrap_or(MAX_ELEMENT_DEPTH));
                Some(self.svg_file(svg, file, inside, counted, reading))
            }
            source => Some(source.map_or_else(|| Outcome::not_read("is not an image VectorCraft reads"), Outcome::shown)),
        }
    }

    /// Linked SVG file `file`, read as `svg`, with the files its own `<image>` elements link to
    /// read from its folder; `inside` is how deep it nests in the SVGs around it, and `counted` the
    /// bytes of it already counted toward [`MAX_NESTED_SVG_TEXT`]. Its text counts toward
    /// [`MAX_NESTED_SVG_TEXT`], and reading its links toward [`MAX_NESTED_SVG_READS`]; once they
    /// are passed, its links aren't read ([`Outcome::capped`], noted when it links to files). Its
    /// links aren't read either when it nests too deep to be shown, or when it isn't XML.
    fn svg_file(&mut self, svg: Arc<SvgFile>, file: &str, inside: usize, counted: u64, reading: &mut Reading<'_, '_>) -> Outcome {
        reading.svg_text = reading.svg_text.saturating_add((svg.text.len() as u64).saturating_sub(counted));
        let links = svg.text.contains("image");
        reading.svg_reads += usize::from(links);
        if reading.spent() {
            if links && links_files(&svg.text) {
                self.too_much_linked.store(true, Ordering::Relaxed);
            }
            return Outcome { source: Ok(Source::Svg(svg)), capped: true };
        }
        let left = MAX_ELEMENT_DEPTH.saturating_sub(inside);
        if !links {
            return Outcome::shown(Source::Svg(svg));
        }
        if depth::xml_deeper_than(&svg.text, left) {
            reading.placed = true;
            return Outcome::shown(Source::Svg(svg));
        }
        // Read from its own handle: the text stays borrowed while `svg` may be given back.
        let text = svg.text.clone();
        let Ok(xml) = super::parse(&text) else { return Outcome::shown(Source::Svg(svg)) };
        // Measured before the style cascade of `depth::height`, as an SVG image is when usvg
        // reads it ([`Files::svg_tree`]), which then refuses one past the limits, within what the
        // linked SVG files measured before it left: past it, its links aren't read, and the
        // selector tests it made still count.
        let (result, tests) = budget::check_spending(&xml, budget::Cost::LIMITS.less(reading.measured), left);
        match result {
            Ok(cost) => reading.measured = reading.measured.add(cost),
            Err(e) => {
                reading.measured.tests = reading.measured.tests.saturating_add(tests);
                reading.placed |= matches!(e, SvgError::TooDeep(_));
                return Outcome::shown(Source::Svg(svg));
            }
        }
        let Some(height) = depth::height(&xml, left) else {
            reading.placed = true;
            return Outcome::shown(Source::Svg(svg));
        };
        let folder = Path::new(file).parent().map(|p| p.to_string_lossy().into_owned());
        let mut edits = Edits::default();
        reading.active.push(identity(file));
        let sources = self.links_of(&text, &xml, folder.as_deref(), Some(inside + height), &mut edits, reading);
        reading.active.pop();
        let text = Arc::from(edits.apply(&text).as_ref());
        Outcome::shown(Source::Svg(Arc::new(SvgFile { text, sources })))
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
        if self.too_much_linked.load(Ordering::Relaxed) {
            notes.push(format!(
                "images linked from linked SVG files were left out: once {MAX_NESTED_SVG_READS} SVG files or {} MB have been read for their links, the files they link to aren't read",
                MAX_NESTED_SVG_TEXT >> 20
            ));
        }
        if self.too_much_data.load(Ordering::Relaxed) {
            notes.push(format!("images were left out: together their data URLs decode to more than {} MB", MAX_DATA_URL_BYTES >> 20));
        }
        notes
    }

    /// What usvg makes of the SVG holding the images ([`budget::check`]): its images make at most
    /// what is left of [`budget::Cost::LIMITS`].
    pub(in crate::import) fn spent_by_svg(&self, spent: budget::Cost) {
        self.set_spent(spent);
    }

    fn spent(&self) -> budget::Cost {
        self.spent.lock().map(|s| *s).unwrap_or(budget::Cost::LIMITS)
    }

    fn set_spent(&self, spent: budget::Cost) {
        if let Ok(mut s) = self.spent.lock() {
            *s = spent;
        }
    }

    /// The error of the first SVG image past [`budget`]'s limits, which refuses the SVG holding it.
    pub(in crate::import) fn refused(&self) -> Option<SvgError> {
        self.refused.lock().ok().and_then(|r| r.clone())
    }

    fn note_text(&self, svg: &str) {
        if svg.contains("<text") {
            self.text.store(true, Ordering::Relaxed);
        }
    }

    /// usvg's image resolver, for an SVG whose elements nest `height` deep: data URLs by their
    /// keys ([`Files::data_url`]), and those usvg decoded itself as it reads them, within
    /// [`MAX_DATA_URL_BYTES`]; other hrefs only from the files read. SVG images are read here
    /// ([`Files::svg`]), never by usvg's own resolver, which caps neither how deep they nest nor
    /// what they unpack to.
    pub(in crate::import) fn resolver(&self, height: usize) -> ImageHrefResolver<'_> {
        self.depth.store(height, Ordering::Relaxed);
        ImageHrefResolver {
            resolve_data: Box::new(move |mime, bytes, opts| {
                let spent = self.data_bytes.load(Ordering::Relaxed).saturating_add(bytes.len() as u64);
                if spent > MAX_DATA_URL_BYTES {
                    return leave_out(&self.too_much_data, true);
                }
                self.data_bytes.store(spent, Ordering::Relaxed);
                self.show(self.decoded(mime, bytes), opts)
            }),
            resolve_string: Box::new(move |h, opts| {
                let link = match self.data_number(h).map(|n| self.data_url(n, opts)) {
                    Some(Ok(image)) => return image,
                    Some(Err(link)) => link,
                    None => Arc::from(h),
                };
                match self.find(&link)? {
                    Source::Raster(kind, bytes) => Some(kind(bytes)),
                    Source::Svg(file) => self.svg(&file.text.clone(), Some(file), opts),
                }
            }),
        }
    }

    /// The number of the href whose key is `h`.
    fn data_number(&self, h: &str) -> Option<usize> {
        let n: usize = h.strip_prefix(DATA_URL_KEY)?.parse().ok()?;
        let known = n < self.data.lock().ok()?.urls.len();
        (known && h.len() == DATA_URL_KEY.len() + n.to_string().len()).then_some(n)
    }

    /// The href whose key is `h`, as it was written: a filter kept as an effect holds its
    /// `<feImage>` hrefs as written, not their keys ([`super::fx::sources`]).
    pub(in crate::import) fn data_href(&self, h: &str) -> Option<Arc<str>> {
        let n = self.data_number(h)?;
        self.data.lock().ok()?.hrefs.get(n).cloned()
    }

    /// The image of data URL `n`: decoded the first time ([`Files::decode`]), then kept; the link
    /// when href `n` is one.
    fn data_url(&self, n: usize, opts: &usvg::Options) -> Result<Option<ImageKind>, Arc<str>> {
        // The lock is let go before the image is shown: an SVG image takes it again.
        let state = match self.data.lock().ok().as_deref().and_then(|d| d.urls.get(n)) {
            Some(DataUrl::Decoded(decoded)) => Ok(decoded.clone()),
            Some(DataUrl::Encoded(url)) => Err(url.clone()),
            Some(DataUrl::Link(link)) => return Err(link.clone()),
            None => return Ok(None),
        };
        let url = match state {
            Ok(decoded) => return Ok(self.show(decoded, opts)),
            Err(url) => url,
        };
        let decoded = self.decode(&url);
        if let Ok(mut data) = self.data.lock()
            && let Some(slot) = data.urls.get_mut(n)
        {
            *slot = DataUrl::Decoded(decoded.clone());
        }
        Ok(self.show(decoded, opts))
    }

    /// What data URL `url` decodes to, as usvg decodes it, in what is left of
    /// [`MAX_DATA_URL_BYTES`]: decoding stops where it would go past, the image left out, noted.
    fn decode(&self, url: &str) -> Decoded {
        let Ok(parsed) = data_url::DataUrl::process(url) else { return Decoded::Nothing };
        let spent = self.data_bytes.load(Ordering::Relaxed);
        let room = MAX_DATA_URL_BYTES.saturating_sub(spent);
        let mut body = Vec::new();
        let decoded = parsed.decode(|bytes| {
            if (body.len() as u64).saturating_add(bytes.len() as u64) > room {
                return Err(());
            }
            body.extend_from_slice(bytes);
            Ok(())
        });
        self.data_bytes.store(spent.saturating_add(body.len() as u64), Ordering::Relaxed);
        match decoded {
            Ok(_) => {}
            Err(data_url::forgiving_base64::DecodeError::WriteError(())) => {
                self.too_much_data.store(true, Ordering::Relaxed);
                return Decoded::Nothing;
            }
            Err(_) => return Decoded::Nothing,
        }
        let mime = format!("{}/{}", parsed.mime_type().type_, parsed.mime_type().subtype);
        self.decoded(&mime, Arc::new(body))
    }

    /// What usvg gets for data `bytes` of type `mime`: by type as usvg reads them, and TIFF; one
    /// without a type (`text/plain`) by its bytes: one of those rasters, else SVG. A raster as
    /// [`shown`] gives it; SVG or SVGZ as [`Files::svg_text`] gives it.
    fn decoded(&self, mime: &str, bytes: Bytes) -> Decoded {
        use image::ImageFormat as F;
        let format = match mime {
            "image/jpg" | "image/jpeg" => F::Jpeg,
            "image/png" => F::Png,
            "image/gif" => F::Gif,
            "image/webp" => F::WebP,
            "image/tiff" => F::Tiff,
            "image/svg+xml" => return self.svg_text(&bytes),
            "text/plain" => match image::guess_format(&bytes).ok().filter(|f| raster(*f).is_some() || *f == F::Tiff) {
                Some(format) => format,
                None => return self.svg_text(&bytes),
            },
            _ => return Decoded::Nothing,
        };
        match shown(bytes, format) {
            Some(shown) => {
                let (kind, bytes) = self.keep(shown);
                Decoded::Raster(kind, bytes)
            }
            None => Decoded::Nothing,
        }
    }

    /// The text of SVG image data `bytes` (SVG or SVGZ). Unpacked here, within what the SVGZ
    /// images have left, so usvg unpacks nothing itself; past it, nothing, noted.
    fn svg_text(&self, bytes: &[u8]) -> Decoded {
        match crate::text_within(bytes, &self.unpacked) {
            Ok(text) => Decoded::Svg(Arc::from(text.as_ref())),
            Err(e) => {
                leave_out(&self.too_big, matches!(e, SvgError::TooBig(_)));
                Decoded::Nothing
            }
        }
    }

    /// The image usvg gets for `decoded`: an SVG image read here, where it is shown.
    fn show(&self, decoded: Decoded, opts: &usvg::Options) -> Option<ImageKind> {
        match decoded {
            Decoded::Raster(kind, bytes) => Some(kind(bytes)),
            Decoded::Svg(text) => self.svg(&text, None, opts),
            Decoded::Nothing => None,
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
        // The SVG holding it is refused already: no more images are read.
        if self.refused().is_some() {
            return None;
        }
        // Measured as the SVG holding it is (`import`): the text before the XML reader.
        let used = self.depth.load(Ordering::Relaxed);
        let left = MAX_ELEMENT_DEPTH.saturating_sub(used);
        if depth::xml_deeper_than(text, left) {
            return leave_out(&self.too_deep, true);
        }
        let xml = super::parse(text).ok()?;
        // What usvg would make of it, measured as the SVG holding it is (`import`), before the
        // style cascade of `depth::height`; its elements, copied styles and selector tests count
        // with theirs while usvg reads it, and only when it decodes. Too deep, it is left out;
        // past the other limits, the whole SVG is refused.
        let spent = self.spent();
        let cost = match budget::check(&xml, budget::Cost::LIMITS.less(spent), left) {
            Ok(cost) => cost,
            Err(SvgError::TooDeep(_)) => return leave_out(&self.too_deep, true),
            Err(e) => {
                if let Ok(mut refused) = self.refused.lock() {
                    refused.get_or_insert(e);
                }
                return None;
            }
        };
        let Some(height) = depth::height(&xml, left) else { return leave_out(&self.too_deep, true) };
        // usvg's parser would overflow on it, as on the SVG holding it (`import`).
        super::convolve_matrices_ok(&xml).ok()?;
        self.set_spent(spent.add(cost));
        let mut edits = Edits::default();
        self.take_data_urls(&xml, &mut edits);
        let keyed = edits.apply(text);
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
        let tree = usvg::Tree::from_str(&keyed, &nested);
        if within && let Ok(mut w) = self.within.lock() {
            w.pop();
        }
        self.depth.store(used, Ordering::Relaxed);
        let Ok(tree) = tree else {
            // Left out: neither it nor the images inside it count.
            self.set_spent(spent);
            return None;
        };
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
/// and fits the image as it is shown, with the resolution the file declares
/// ([`vectorcraft_doc::ppi::resolution`]), its axes swapped by a quarter turn. A CMYK TIFF also
/// keeps its inks as a CMYK TIFF ([`vectorcraft_doc::cmyk::stored_cmyk_tiff`]), and a CMYK JPEG
/// or TIFF that is turned keeps them turned ([`vectorcraft_doc::cmyk::turned_cmyk`]). `None` when
/// the pixels can't be read, and for a TIFF with more strips or tiles than the decoder can count
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
    let ppi = vectorcraft_doc::ppi::resolution(&bytes);
    // The resolution describes the stored pixels; a quarter turn swaps its axes.
    let quarter = orientation.is_some_and(|o| o.to_exif() >= 5);
    let shown_ppi = ppi.map(|(x, y)| if quarter { (y, x) } else { (x, y) });
    let cmyk = match orientation {
        Some(o) if matches!(format, F::Jpeg | F::Tiff) => vectorcraft_doc::cmyk::turned_cmyk(&bytes, o, shown_ppi).map(|(tiff, _)| tiff),
        None if tiff => vectorcraft_doc::cmyk::stored_cmyk_tiff(&bytes, ppi),
        _ => None,
    };
    if let Some(orientation) = orientation {
        img.apply_orientation(orientation);
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), F::Png).ok()?;
    if let Some(ppi) = shown_ppi {
        png = vectorcraft_doc::ppi::with_png_resolution(&png, ppi);
    }
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

    /// An SVG file of 2 × 2 showing `hrefs`.
    fn linking(hrefs: &[&str]) -> Vec<u8> {
        let images: String = hrefs.iter().map(|h| format!(r#"<image href="{h}" width="1" height="1"/>"#)).collect();
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2">{images}</svg>"#).into_bytes()
    }

    /// The links of the SVG showing `hrefs` read as [`Files::read`] reads them, the files `served`
    /// by name, with the SVG files and bytes read for links starting at `reads` and `text`: the
    /// names of the files read, in order, and the files.
    fn read_links(hrefs: &[&str], served: &[(&str, Vec<u8>)], reads: usize, text: u64) -> (Vec<String>, Files) {
        let log = Mutex::new(vec![]);
        let read = |path: &str| {
            let name = Path::new(path).file_name()?.to_string_lossy().into_owned();
            log.lock().ok()?.push(name.clone());
            served.iter().find(|(n, _)| *n == name).map(|(_, bytes)| (bytes.clone(), LinkInfo::new(path)))
        };
        let folder = std::env::temp_dir().join("vectorcraft-svg-limits");
        let folder = folder.to_string_lossy();
        let opts = ImportOptions { folder: Some(&folder), read: Some(&read) };
        let svg = String::from_utf8(linking(hrefs)).unwrap();
        let xml = super::super::parse(&svg).unwrap();
        let mut warnings = vec![];
        let mut reading = Reading {
            opts: &opts,
            warnings: &mut warnings,
            placeholder: None,
            by_file: HashMap::new(),
            active: vec![],
            placed: false,
            svg_reads: reads,
            svg_text: text,
            measured: budget::Cost::default(),
        };
        let mut files = Files::default();
        files.sources = files.links_of(&svg, &xml, opts.folder, None, &mut Edits::default(), &mut reading);
        (log.into_inner().unwrap(), files)
    }

    /// The linked SVG files measured for their links share the limits: each of these keeps about
    /// 40 MB of declarations in the import's own cascade (its `:not()` rule), so the second one is
    /// shown without its links, and neither it nor the cascade reads them.
    #[test]
    fn linked_svg_files_share_the_limits_for_their_links() {
        let file = |png: &str| {
            let groups = "<g/>".repeat(400);
            let rule = format!("g:not(.x){{font-family:{}}}", "a".repeat(100_000));
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><style>{rule}</style>{groups}<image href="{png}" width="1" height="1"/></svg>"#)
                .into_bytes()
        };
        let served = [("a.svg", file("a.png")), ("b.svg", file("b.png")), ("a.png", png()), ("b.png", png())];
        let (log, _) = read_links(&["a.svg"], &served, 0, 0);
        assert_eq!(log, ["a.svg", "a.png"]);
        let (log, _) = read_links(&["a.svg", "b.svg"], &served, 0, 0);
        assert_eq!(log, ["a.svg", "a.png", "b.svg"]);
    }

    /// A linked SVG file past the limits alone (a `<use>` chain past the element limit) is shown
    /// without its links, and the linked SVG files after it still get what it left.
    #[test]
    fn a_linked_svg_file_past_the_limits_leaves_the_others_their_links() {
        let links: String = (1..=20).map(|i| format!(r##"<g id="g{i}"><use href="#g{0}"/><use href="#g{0}"/></g>"##, i - 1)).collect();
        let chain = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><defs><rect id="g0" width="1" height="1"/>{links}</defs><use href="#g20"/><image href="a.png" width="1" height="1"/></svg>"##
        );
        let served = [("a.svg", chain.into_bytes()), ("b.svg", linking(&["b.png"])), ("a.png", png()), ("b.png", png())];
        let (log, _) = read_links(&["a.svg", "b.svg"], &served, 0, 0);
        assert_eq!(log, ["a.svg", "b.svg", "b.png"]);
    }

    fn png() -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::new(1, 1).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn linked_note() -> String {
        format!(
            "images linked from linked SVG files were left out: once {MAX_NESTED_SVG_READS} SVG files or {} MB have been read for their links, the files they link to aren't read",
            MAX_NESTED_SVG_TEXT >> 20
        )
    }

    /// Every file read for a linked SVG file's links counts toward [`MAX_NESTED_SVG_TEXT`], a
    /// raster too, after the linked SVG file's own text: the file that passes it is left out, and
    /// no file after it is read.
    #[test]
    fn linked_files_past_the_text_limit_are_not_read() {
        let a = linking(&["one.png", "two.png", "three.png"]);
        let served = [("a.svg", a.clone()), ("one.png", png()), ("two.png", png()), ("three.png", png())];
        let (log, files) = read_links(&["a.svg"], &served, 0, MAX_NESTED_SVG_TEXT - (png().len() + a.len()) as u64);
        assert_eq!(log, ["a.svg", "one.png", "two.png"]);
        let Some(Source::Svg(a)) = files.sources.get("a.svg") else { panic!("a.svg is shown") };
        assert!(a.sources.contains_key("one.png") && a.sources.len() == 1, "{:?}", a.sources.keys());
        assert_eq!(files.left_out(), [linked_note()]);
    }

    /// Once more than [`MAX_NESTED_SVG_READS`] SVG files are read for their links, the SVG file
    /// past it is left out inside a linked SVG file, and no file a linked SVG file links to is
    /// read (a nested link past the limit); rasters the imported SVG links to still are.
    #[test]
    fn linked_files_past_the_read_limit_are_not_read() {
        let served = [("a.svg", linking(&["b.svg", "two.png"])), ("b.svg", linking(&["one.png"])), ("one.png", png()), ("two.png", png())];
        let (log, files) = read_links(&["a.svg", "two.png"], &served, MAX_NESTED_SVG_READS - 1, 0);
        assert_eq!(log, ["a.svg", "b.svg", "two.png"]);
        let Some(Source::Svg(a)) = files.sources.get("a.svg") else { panic!("a.svg is shown") };
        assert!(a.sources.is_empty(), "{:?}", a.sources.keys());
        assert!(matches!(files.sources.get("two.png"), Some(Source::Raster(..))));
        assert_eq!(files.left_out().len(), 2, "{:?}", files.left_out());
        assert!(files.left_out().contains(&linked_note()));
    }

    /// Reading the SVG files the imported SVG links to, and their links, counts toward the
    /// limits: past them, such a file is still shown, but the files it links to aren't read.
    #[test]
    fn svg_files_the_imported_svg_links_to_are_shown_without_their_links_past_the_limits() {
        let served = [("a.svg", linking(&["one.png"])), ("one.png", png())];
        for (reads, text) in [(MAX_NESTED_SVG_READS, 0), (0, MAX_NESTED_SVG_TEXT)] {
            let (log, files) = read_links(&["a.svg"], &served, reads, text);
            assert_eq!(log, ["a.svg"], "one.png isn't read");
            let Some(Source::Svg(a)) = files.sources.get("a.svg") else { panic!("a.svg is shown") };
            assert!(a.sources.is_empty(), "{:?}", a.sources.keys());
            assert_eq!(files.left_out(), [linked_note()]);
        }
        // More of them than the limit: those past it are shown without their links.
        let names: Vec<String> = (0..MAX_NESTED_SVG_READS + 2).map(|i| format!("s{i}.svg")).collect();
        let hrefs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut served: Vec<(&str, Vec<u8>)> = hrefs.iter().map(|h| (*h, linking(&["one.png"]))).collect();
        served.push(("one.png", png()));
        let (log, files) = read_links(&hrefs, &served, 0, 0);
        assert_eq!(log.len(), hrefs.len() + 1, "one.png is read once");
        let with_links = hrefs.iter().filter(|h| matches!(files.sources.get(**h), Some(Source::Svg(s)) if s.sources.contains_key("one.png")));
        assert_eq!(with_links.count(), MAX_NESTED_SVG_READS);
        assert!(hrefs.iter().all(|h| matches!(files.sources.get(*h), Some(Source::Svg(_)))), "all shown");
        assert_eq!(files.left_out(), [linked_note()]);
    }

    /// An SVG file read once is kept by its file wherever it was read: another spelling of its
    /// href doesn't read it again, past the limits too.
    #[test]
    fn an_svg_file_is_read_once_past_the_limits() {
        let served = [("a.svg", linking(&["one.png"])), ("one.png", png())];
        let (log, files) = read_links(&["a.svg", "./a.svg", "%61.svg"], &served, MAX_NESTED_SVG_READS, 0);
        assert_eq!(log, ["a.svg"]);
        assert!(["a.svg", "./a.svg", "%61.svg"].iter().all(|h| matches!(files.sources.get(*h), Some(Source::Svg(_)))));
        // Read inside a linked SVG file, where its read passes the text limit: left out there,
        // and shown, not read again, where the imported SVG links to it.
        let x = linking(&["a.svg"]);
        let served = [("x.svg", x.clone()), ("a.svg", linking(&["one.png"])), ("one.png", png())];
        let (log, files) = read_links(&["x.svg", "./a.svg"], &served, 0, MAX_NESTED_SVG_TEXT - x.len() as u64);
        assert_eq!(log, ["x.svg", "a.svg"]);
        let Some(Source::Svg(x)) = files.sources.get("x.svg") else { panic!("x.svg is shown") };
        assert!(x.sources.is_empty(), "{:?}", x.sources.keys());
        let Some(Source::Svg(a)) = files.sources.get("./a.svg") else { panic!("a.svg is shown") };
        assert!(a.sources.is_empty(), "{:?}", a.sources.keys());
        assert_eq!(files.left_out().len(), 2, "{:?}", files.left_out());
    }

    /// A base64 data URL of `bytes` of type `mime`.
    fn data(mime: &str, bytes: &[u8]) -> String {
        format!("data:{mime};base64,{}", crate::base64_encode(bytes))
    }

    /// A PNG of `w` × 1.
    fn png_of(w: u32) -> Vec<u8> {
        let mut out = vec![];
        image::RgbaImage::new(w, 1).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// SVG text of 2 × 2 holding `body`.
    fn doc(body: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="2" height="2">{body}</svg>"#)
    }

    /// `svg` read as the import reads it, its data URLs changed to keys first, `spent` bytes of
    /// [`MAX_DATA_URL_BYTES`] already decoded: the files, and the PNG bytes of the images usvg
    /// made, filters' included, in order.
    fn read_keyed(svg: &str, spent: u64) -> (Files, Vec<Arc<Vec<u8>>>) {
        let xml = super::super::parse(svg).unwrap();
        let mut edits = Edits::default();
        let files = Files::read(svg, &xml, &ImportOptions::default(), &mut edits, &mut vec![]);
        files.data_bytes.store(spent, Ordering::Relaxed);
        let text = edits.apply(svg).into_owned();
        let tree = {
            let height = depth::height(&xml, MAX_ELEMENT_DEPTH).unwrap();
            let opts = usvg::Options { image_href_resolver: files.resolver(height), ..usvg::Options::default() };
            usvg::Tree::from_str(&text, &opts).unwrap()
        };
        // Filters' images and SVG images' trees are subroots.
        fn pngs(g: &usvg::Group, out: &mut Vec<Arc<Vec<u8>>>) {
            for n in g.children() {
                match n {
                    usvg::Node::Group(g) => pngs(g, out),
                    usvg::Node::Image(im) => {
                        if let ImageKind::PNG(b) = im.kind() {
                            out.push(b.clone());
                        }
                    }
                    _ => {}
                }
                n.subroots(|g| pngs(g, out));
            }
        }
        let mut out = vec![];
        pngs(tree.root(), &mut out);
        (files, out)
    }

    /// A data URL shown by many `<use>` elements is decoded once, and every use gets the same
    /// image, where usvg decoded it again for each.
    #[test]
    fn a_data_url_is_decoded_once_however_often_it_is_used() {
        let png = png();
        let uses = r##"<use href="#p"/>"##.repeat(64);
        let svg = doc(&format!(r#"<defs><image id="p" href="{}" width="1" height="1"/></defs>{uses}"#, data("image/png", &png)));
        let (files, shown) = read_keyed(&svg, 0);
        assert_eq!(shown.len(), 64);
        assert!(shown.iter().all(|b| Arc::ptr_eq(b, &shown[0]) && **b == png));
        assert_eq!(files.data_bytes.load(Ordering::Relaxed), png.len() as u64);
        assert!(files.left_out().is_empty(), "{:?}", files.left_out());
    }

    /// The data URLs inside an SVG image get keys too: each is decoded once, together with the
    /// SVG image's own URL.
    #[test]
    fn data_urls_in_svg_images_are_decoded_once() {
        let png = png();
        let uses = r##"<use href="#p"/>"##.repeat(16);
        let inner = doc(&format!(r#"<defs><image id="p" href="{}" width="1" height="1"/></defs>{uses}"#, data("image/png", &png)));
        let svg = doc(&format!(r#"<image href="{}" width="2" height="2"/>"#, data("image/svg+xml", inner.as_bytes())));
        let (files, shown) = read_keyed(&svg, 0);
        assert_eq!(shown.len(), 16);
        assert_eq!(files.data_bytes.load(Ordering::Relaxed), (inner.len() + png.len()) as u64);
    }

    /// An `<feImage>`'s data URL gets a key too.
    #[test]
    fn fe_image_data_urls_are_decoded_once() {
        let png = png();
        let rects: String = (1..=8).map(|i| format!(r#"<rect width="{i}" height="1" filter="url(#f)"/>"#)).collect();
        let svg = doc(&format!(r#"<filter id="f"><feImage href="{}"/></filter>{rects}"#, data("image/png", &png)));
        let (files, shown) = read_keyed(&svg, 0);
        assert_eq!(shown.len(), 8);
        assert_eq!(files.data_bytes.load(Ordering::Relaxed), png.len() as u64);
    }

    /// Both hrefs get keys, and usvg still takes the plain `href` over `xlink:href`, in either
    /// order: only its URL is decoded.
    #[test]
    fn the_plain_href_still_wins_over_xlink_href() {
        let (one, two) = (png_of(1), png_of(2));
        let (one_url, two_url) = (data("image/png", &one), data("image/png", &two));
        for image in [
            format!(r#"<image href="{one_url}" xlink:href="{two_url}" width="1" height="1"/>"#),
            format!(r#"<image xlink:href="{two_url}" href="{one_url}" width="1" height="1"/>"#),
        ] {
            let (files, shown) = read_keyed(&doc(&image), 0);
            assert_eq!(shown.len(), 1);
            assert_eq!(*shown[0], one);
            assert_eq!(files.data_bytes.load(Ordering::Relaxed), one.len() as u64);
        }
    }

    /// An image whose data URL is written in an entity, and so in every element the entity
    /// makes, gets a key too.
    #[test]
    fn data_urls_written_in_entities_are_decoded_once() {
        let png = png();
        let uses = r##"<use href="#p"/>"##.repeat(16);
        let svg = format!(
            r#"<!DOCTYPE svg [<!ENTITY img "<image id='p' href='{}' width='1' height='1'/>">]>{}"#,
            data("image/png", &png),
            doc(&format!("<defs>&img;</defs>{uses}"))
        );
        let (files, shown) = read_keyed(&svg, 0);
        assert_eq!(shown.len(), 16);
        assert_eq!(files.data_bytes.load(Ordering::Relaxed), png.len() as u64);
    }

    /// The data URLs decode to at most [`MAX_DATA_URL_BYTES`] together: a URL that fits what is
    /// left is decoded, and decoding stops where a URL would go past it, that image left out and
    /// noted.
    #[test]
    fn data_urls_past_the_limit_are_left_out() {
        let (small, big) = (png(), png_of(64));
        assert!(small.len() < big.len());
        let svg = doc(&format!(
            r#"<image href="{}" width="1" height="1"/><image href="{}" width="1" height="1"/>"#,
            data("image/png", &small),
            data("image/png", &big)
        ));
        let spent = MAX_DATA_URL_BYTES - (small.len() + big.len()) as u64 + 1;
        let (files, shown) = read_keyed(&svg, spent);
        assert_eq!(shown.len(), 1);
        assert_eq!(*shown[0], small);
        assert!(files.data_bytes.load(Ordering::Relaxed) <= MAX_DATA_URL_BYTES);
        assert_eq!(files.left_out(), [format!("images were left out: together their data URLs decode to more than {} MB", MAX_DATA_URL_BYTES >> 20)]);
        // A data URL usvg decodes itself counts too.
        let files = Files::default();
        files.data_bytes.store(MAX_DATA_URL_BYTES, Ordering::Relaxed);
        let resolver = files.resolver(1);
        assert!((resolver.resolve_data)("image/png", Arc::new(small.clone()), &usvg::Options::default()).is_none());
        drop(resolver);
        assert_eq!(files.left_out().len(), 1);
    }

    /// A link that looks like a key, spelled plainly or with character references, in the
    /// imported SVG or in an SVG image, stays that link: it never shows a data URL's image.
    #[test]
    fn links_that_look_like_keys_stay_links() {
        let png = png();
        let url = data("image/png", &png);
        let inner = doc(&format!(r#"<image href="{DATA_URL_KEY}0" width="1" height="1"/>"#));
        let svg = doc(&format!(
            r#"<image href="{url}" width="1" height="1"/><image href="{DATA_URL_KEY}0" width="1" height="1"/><image href="&#118;{}0" width="1" height="1"/><image href="{}" width="2" height="2"/>"#,
            &DATA_URL_KEY[1..],
            data("image/svg+xml", inner.as_bytes())
        ));
        let (files, shown) = read_keyed(&svg, 0);
        // The data URL's image, then the placeholder of the missing file, twice; the SVG image's
        // link names no file there, so it shows nothing.
        assert_eq!(shown.len(), 3);
        assert_eq!(*shown[0], png);
        assert!(shown[1..].iter().all(|b| **b != png && files.links.get(&key(b)).is_some_and(|l| l.missing)));
        assert_eq!(files.data_number(&format!("{DATA_URL_KEY}00")), None);
        assert_eq!(files.data_number(&format!("{DATA_URL_KEY}+0")), None);
        assert_eq!(files.data_number(&format!("{DATA_URL_KEY}9")), None);
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
