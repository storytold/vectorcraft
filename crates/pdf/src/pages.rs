//! The pages of a PDF to import: opening with a password, the pages picked, the box each page
//! is cropped to ([`CropTo`]) and [`info`].

use std::collections::{HashMap, HashSet};

use craft::pdf::syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream, dict_or_stream};
use craft::pdf::syntax::page::Page;
use craft::pdf::syntax::xref::XRef;
use craft::pdf::syntax::{DecryptionError, LoadPdfError, Pdf};
use kurbo::{Affine, Rect};
use vectorcraft_doc::setup::MAX_BLEED;

use crate::import_color::{direct, items, readable, values};
use crate::streams::{Refusal, Refused, refused};
use crate::{Choice, CropTo, ImportOptions, PdfError};

/// What `info` tells about one page.
#[derive(Clone, Debug, PartialEq)]
pub struct PageInfo {
    /// The page's size in points as viewers show it (its crop box, rotated, times the `UserUnit`
    /// the PDF reader applies to the page).
    pub width: f64,
    pub height: f64,
    /// The page's rotation in degrees (0, 90, 180 or 270).
    pub rotation: u16,
    /// Every box but Bounding, in PDF user space (y up), clipped to the media box, times the same
    /// `UserUnit` (in points, as the size). A missing bleed, trim or art box is the crop box.
    pub boxes: Vec<(CropTo, Rect)>,
}

/// The pages of a PDF ([`info`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PdfInfo {
    pub pages: Vec<PageInfo>,
}

/// Is this a PostScript file (plain, or an EPS with a binary header) rather than a PDF?
pub fn is_postscript(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%!PS") || bytes.starts_with(&[0xC5, 0xD0, 0xD3, 0xC6])
}

/// The application's stricter refusal policy for functions and colour spaces built on one
/// another (a function stitching functions together, an indexed space on an alternate).
/// The shared interpreter also has function depth and node budgets; this preflight rejects
/// cyclic or excessively linked objects before interpretation. Files usually have two or three.
const MAX_LINKS: usize = 16;

/// The application's calculator-procedure depth policy, layered over the shared reader's limits.
const MAX_PROCEDURES: usize = 64;

/// What the function or colour space `o` is built on: the functions a stitching function joins;
/// the base, alternate and tint of a colour space; the alternate of a colour profile. Nothing for
/// any other object.
fn links<'a>(o: &Object<'a>, xref: &'a XRef) -> Vec<MaybeRef<Object<'a>>> {
    const SPACES: [&[u8]; 6] = [b"Indexed", b"I", b"Separation", b"DeviceN", b"Pattern", b"ICCBased"];
    if let Object::Array(a) = o {
        let mut items = items(a);
        let kind = match items.next() {
            Some(MaybeRef::NotRef(Object::Name(n))) => Some(n),
            Some(MaybeRef::Ref(r)) => xref.get::<Name<'a>>(r.into()),
            _ => None,
        };
        return if kind.is_some_and(|n| SPACES.contains(&n.as_ref())) { items.collect() } else { vec![] };
    }
    let Some((d, _)) = dict_or_stream(o) else { return vec![] };
    let functions = d.get::<Array<'a>>(b"Functions").map(|a| items(&a).collect::<Vec<_>>()).unwrap_or_default();
    functions.into_iter().chain(d.get_raw::<Object<'a>>(b"Alternate")).collect()
}

/// How many of its kind the function or colour space `o` is built on, one on the next (0 for any
/// other object); `None` past `room`, or when it is built on itself. `known`: the objects looked
/// at so far (`None` while one is being looked at, too). One `None` refuses the file, so that an
/// object is `None` only for the room there was where it was first met doesn't matter.
fn built_on<'a>(o: MaybeRef<Object<'a>>, xref: &'a XRef, known: &mut HashMap<ObjectIdentifier, Option<usize>>, room: usize) -> Option<usize> {
    let (id, o) = match o {
        MaybeRef::Ref(r) => {
            let id = ObjectIdentifier::from(r);
            if let Some(found) = known.get(&id) {
                return *found;
            }
            let Some(o) = xref.get::<Object<'a>>(id) else { return Some(0) };
            known.insert(id, None);
            (Some(id), o)
        }
        MaybeRef::NotRef(o) => (None, o),
    };
    let links = links(&o, xref);
    let most = match room.checked_sub(1) {
        _ if links.is_empty() => Some(0),
        Some(room) => links.into_iter().try_fold(0, |most, on| built_on(on, xref, known, room).map(|n| most.max(n + 1))),
        None => None,
    };
    // (What it is built on may be known already, and deeper than there is room for here.)
    let most = most.filter(|n| *n <= room);
    if let Some(id) = id {
        known.insert(id, most);
    }
    most
}

/// Is `o` a calculator function whose procedures nest deeper than [`MAX_PROCEDURES`]?
fn procedures_too_deep(o: &Object<'_>) -> bool {
    let Object::Stream(s) = o else { return false };
    if s.dict().get::<u8>(b"FunctionType") != Some(4) {
        return false;
    }
    let Ok(program) = s.decoded() else { return false };
    let mut depth = 0usize;
    program.iter().any(|c| {
        match c {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        depth > MAX_PROCEDURES
    })
}

/// The error of a file with an object or cross-reference stream the reader would fail on as it
/// opens the file ([`crate::streams::unsafe_file_streams`]); [`stream_note`] names any other.
const STREAM_NOTE: &str = "a stream in it would decode to more than 256 MB, or its decode parameters can't be applied";

/// The error of a file with functions or color spaces built on one another past [`MAX_LINKS`]
/// (or on themselves), or a calculator function nested past [`MAX_PROCEDURES`].
const NESTED_NOTE: &str = "its functions or colour spaces are built on one another without end";

/// The error of a file with an array whose items can't all be read (a damaged file).
const UNREADABLE_NOTE: &str = "an array in it is damaged and can't be read";

/// Why `pdf` is refused, if it is: a stream the reader would fail on ([`stream_note`]), or
/// functions, color spaces or calculator procedures nested too deep ([`NESTED_NOTE`]). Every
/// object is looked at, and every array, dictionary and stream written inside one, wherever it
/// is used from: the interpreter builds functions and color spaces from resources, shadings,
/// images, patterns, masks and graphics states alike.
fn hostile(pdf: &Pdf) -> Option<String> {
    let xref = pdf.xref();
    let mut known = HashMap::new();
    // The objects of the file, then what each holds directly (64 deep at most: the reader's cap).
    let mut pending: Vec<Object<'_>> = pdf.objects().into_iter().collect();
    while let Some(o) = pending.pop() {
        // A stream's own checks come first: a calculator function is decoded below.
        if let Object::Stream(s) = &o
            && let Some(r) = refused(s)
        {
            return Some(stream_note(pdf, s, r));
        }
        if procedures_too_deep(&o) || built_on(MaybeRef::NotRef(o.clone()), xref, &mut known, MAX_LINKS).is_none() {
            return Some(NESTED_NOTE.into());
        }
        match &o {
            // An array with an item that can't be read can't be looked at whole, and the
            // interpreter would still read its leading items: the file is refused.
            Object::Array(a) if !readable(a) => return Some(UNREADABLE_NOTE.into()),
            Object::Array(a) => pending.extend(items(a).filter_map(direct)),
            _ => pending.extend(dict_or_stream(&o).into_iter().flat_map(|(d, _)| values(d)).filter_map(direct)),
        }
    }
    None
}

/// The error of a file with the stream `s` the reader would fail on, as [`refused`] says why:
/// which object it is and what it holds, so a report of a file that can't be shared still says
/// which stream it is (#4366).
fn stream_note(pdf: &Pdf, s: &Stream<'_>, r: Refusal) -> String {
    let id = s.obj_id();
    // A stream written inside another object goes by that object; 0 is a stream the reader can't
    // place.
    let object = (id.obj_number > 0).then(|| format!("object {}", id.obj_number));
    let what = match (r.inline_image, object) {
        (true, Some(object)) => format!("an inline image in the content of {object}"),
        (true, None) => "an inline image in it".to_string(),
        (false, Some(object)) => format!("{} ({object})", stream_kind(pdf, id, s.dict())),
        (false, None) => stream_kind(pdf, id, s.dict()),
    };
    match r.why {
        Refused::TooBig(most) => format!("{what} would decode to more than {}", decoded_size(most)),
        Refused::Parameters => format!("{what} has decode parameters that can't be applied"),
    }
}

/// What the stream `id` with dictionary `dict` holds, for [`stream_note`]: an image (and its size
/// in pixels), a form, a page's content, the editing data an `.ai` keeps in its first page's
/// `PieceInfo`, or another stream.
fn stream_kind(pdf: &Pdf, id: ObjectIdentifier, dict: &Dict<'_>) -> String {
    let subtype = dict.get::<Name<'_>>(b"Subtype");
    let subtype_is = |v: &[u8]| subtype.as_ref().is_some_and(|n| n.as_ref() == v);
    if subtype_is(b"Image") {
        let side = |key: &[u8]| dict.get::<u32>(key).unwrap_or(0);
        return format!("an image of {} × {} px in it", side(b"Width"), side(b"Height"));
    }
    if subtype_is(b"Form") {
        return "a form in it".to_string();
    }
    let is = |m: Option<MaybeRef<Object<'_>>>| matches!(m, Some(MaybeRef::Ref(r)) if ObjectIdentifier::from(r) == id);
    let pages = pdf.pages();
    // `Contents` is a stream or an array of them, the array in the page or an object of its own.
    // Each object is read once however many pages share it, so a file can't make this quadratic.
    let mut read = HashSet::new();
    let content = pages.iter().any(|page| match page.raw().get_raw::<Object<'_>>(b"Contents") {
        Some(MaybeRef::NotRef(Object::Array(a))) => items(&a).any(|m| is(Some(m))),
        Some(MaybeRef::Ref(r)) => {
            let at = ObjectIdentifier::from(r);
            read.insert(at) && page.raw().get::<Array<'_>>(b"Contents").map_or(at == id, |a| items(&a).any(|m| is(Some(m))))
        }
        _ => false,
    });
    if content {
        return "a page's content in it".to_string();
    }
    let private = pages.first().and_then(|p| p.raw().get::<Dict<'_>>(b"PieceInfo")?.get::<Dict<'_>>(PRIVATE_DATA_OWNER)?.get::<Dict<'_>>(b"Private"));
    if private.is_some_and(|d| d.entries().any(|(key, m)| key.as_ref().starts_with(b"AI") && is(Some(m)))) {
        return "its editing data".to_string();
    }
    "a stream in it".to_string()
}

/// `bytes` for a message, in whole megabytes, or gigabytes when it is a whole number of them.
pub(crate) fn decoded_size(bytes: usize) -> String {
    const GB: usize = 1 << 30;
    if bytes >= GB && bytes.is_multiple_of(GB) { format!("{} GB", bytes / GB) } else { format!("{} MB", bytes >> 20) }
}

/// Read a PDF, decrypting it with `password` (none: the empty user password). The permissions
/// password opens a file too.
///
/// What a hostile file nests or loops is the shared reader's to refuse (it reads arrays and
/// dictionaries 64 deep and no deeper, wherever they are: the file's text, an object stream, the
/// content of a page, form, pattern or glyph; and follows a page tree or a chain of object streams
/// once). What it reads beyond that is left out, as an object it can't read. The application also
/// rejects overly linked functions and colour spaces, oversized streams and unsupported
/// predictors before interpretation ([`hostile`], [`crate::streams`]), retaining its stricter
/// refusal policy alongside the shared backend's bounded decoding and construction.
pub(crate) fn open(bytes: &[u8], password: Option<&str>) -> Result<Pdf, PdfError> {
    open_with_options(bytes, password, &open_options())
}

/// A PDF read and checked once ([`open_pdf`]), for reading several things from one file: the
/// document it carries ([`Opened::editing`]), an editor's private data ([`Opened::illustrator_data`]),
/// its File Info ([`Opened::file_info`]), its pages and boxes ([`Opened::info`]) and its art
/// ([`crate::import_opened`]). Each of [`editing_with`](crate::editing_with), [`illustrator_data`],
/// [`crate::file_info`], [`info`] and [`crate::import_with_report`] reads and checks the whole file
/// again.
pub struct Opened {
    pub(crate) pdf: Pdf,
}

/// The PDF `bytes`, read and checked as [`crate::import_with_report`] reads it (with `password`, as
/// [`info`] takes it, and on a thread with the import's stack), to read several things from.
pub fn open_pdf(bytes: &[u8], password: Option<&str>) -> Result<Opened, PdfError> {
    vectorcraft_doc::depth::with_deep_stack(|| open(bytes, password)).map(|pdf| Opened { pdf })
}

impl Opened {
    /// The pages and their boxes, as [`info`] lists them.
    pub fn info(&self) -> PdfInfo {
        info_in(&self.pdf)
    }

    /// The native document the PDF carries, as [`editing`](crate::editing) reads it.
    pub fn editing(&self) -> Option<crate::Editing> {
        crate::editing::editing_in(&self.pdf)
    }

    /// Does the PDF list the editing file among its attachments, also when
    /// [`editing`](Self::editing) can't read it?
    pub fn has_editing_file(&self) -> bool {
        crate::editing::has_editing_file_in(&self.pdf)
    }

    /// Is the PDF's attachment name tree too large to search to its end? It may then list an
    /// editing file that [`editing`](Self::editing) doesn't find, besides the one it reads.
    pub fn editing_search_cut(&self) -> bool {
        crate::editing::editing_search_cut_in(&self.pdf)
    }

    /// The private data of an Illustrator `.ai`, as [`illustrator_data`] reads it.
    pub fn illustrator_data(&self) -> Option<Vec<u8>> {
        private_data(&self.pdf)
    }

    /// What `read` returns, given the private data of an Illustrator `.ai` as one reader over its
    /// streams in order, each decoded as it is read and none joined to the others: no limit on its
    /// size applies but the one `read` keeps. `None` as for [`illustrator_data`](Self::illustrator_data),
    /// also when a stream turns out not to be readable after `read` has started.
    pub fn read_illustrator_data<T>(&self, read: impl FnOnce(&mut dyn std::io::Read) -> T) -> Option<T> {
        streamed_private_data(&self.pdf, read).ok().flatten()
    }

    /// Does the first page carry an editor's copy of the art, as [`has_illustrator_data`] says?
    pub fn has_illustrator_data(&self) -> bool {
        has_private_data_keys(&self.pdf)
    }

    /// The File Info, as [`crate::file_info`] reads it.
    pub fn file_info(&self) -> crate::FileInfo {
        crate::file_info::file_info_in(&self.pdf)
    }

    /// How many pages the PDF has, as [`info`] lists them.
    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// The pages' trim boxes and bleed, as an import of every page on its trim box takes them,
    /// without importing any page's art.
    pub fn trim_pages(&self) -> TrimPages {
        let pages = self.pdf.pages();
        let mut bleed = [0.0; 4];
        for page in pages.iter() {
            for (side, b) in bleed.iter_mut().zip(self::bleed(page)) {
                *side = f64::max(*side, b);
            }
        }
        TrimPages {
            sizes: pages.iter().map(|page| frame(page, CropTo::Trim).1).map(|r| (r.width(), r.height())).collect(),
            user_unit: pages.first().map_or(1.0, |page| f64::from(page.user_unit())),
            bleed,
        }
    }

    /// How far page `index`'s (0-based) bleed box reaches past its trim box, `[top, bottom, left,
    /// right]` in points, as the page shows (rotated); `None` past the last page, and when the bleed
    /// box doesn't hold the trim box.
    pub fn page_bleed(&self, index: usize) -> Option<[f64; 4]> {
        let page = self.pdf.pages().get(index)?;
        let init = Affine::new(page.initial_transform(true).as_coeffs());
        let (trim, bleed) = (init.transform_rect_bbox(page_box(page, CropTo::Trim)), init.transform_rect_bbox(page_box(page, CropTo::Bleed)));
        let sides = [trim.y0 - bleed.y0, bleed.y1 - trim.y1, trim.x0 - bleed.x0, bleed.x1 - trim.x1];
        sides.iter().all(|b| b.is_finite() && *b >= 0.0).then_some(sides)
    }
}

/// A PDF's pages as an import on their trim boxes sizes them ([`Opened::trim_pages`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TrimPages {
    /// Each page's trim box (its crop box when it has none) in points, rotated as the page shows:
    /// width and height.
    pub sizes: Vec<(f64, f64)>,
    /// The size in points of a unit of the first page's user space (its `UserUnit`).
    pub user_unit: f64,
    /// The bleed of the pages, `[top, bottom, left, right]` in points: on each side, the widest
    /// of the pages' bleed, as the import makes it the document's bleed.
    pub bleed: [f64; 4],
}

/// The shared reader's options: files of any size are accepted. Ordinary stream decoding uses the shared reader's immutable per-document limits, and its input
/// copy is fallible. One image may decode to the bytes and pixels of
/// [`crate::streams::image_limits`] for this build, past the shared reader's defaults.
pub(crate) fn open_options() -> craft::pdf::OpenOptions {
    let (bytes, pixels) = crate::streams::image_limits(crate::streams::LARGE_IMAGES);
    let decode_limits = craft::pdf::syntax::DecodeLimits::default().with_max_decoded_image(bytes).with_max_image_pixels(pixels);
    craft::pdf::OpenOptions { max_input_bytes: usize::MAX, decode_limits }
}

pub(crate) fn open_with_options(bytes: &[u8], password: Option<&str>, options: &craft::pdf::OpenOptions) -> Result<Pdf, PdfError> {
    // Twice: looking at every object makes the reader repair a damaged table of objects (once
    // only), and the objects are other ones after that.
    open_structural_with_options(bytes, password, options).and_then(|pdf| match (0..2).find_map(|_| hostile(&pdf)) {
        Some(why) => Err(PdfError::Parse(why)),
        None => Ok(pdf),
    })
}

/// How many times each buffer has been read as a PDF, by its address (tests).
#[cfg(any(test, feature = "read-count"))]
static READS: std::sync::Mutex<std::collections::BTreeMap<usize, usize>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

/// How many times `bytes`, at its address, has been read as a PDF on any thread (tests).
#[cfg(any(test, feature = "read-count"))]
pub fn reads_of(bytes: &[u8]) -> usize {
    READS.lock().ok().and_then(|r| r.get(&(bytes.as_ptr() as usize)).copied()).unwrap_or(0)
}

/// [`open_with_options`] without looking at the file's objects ([`hostile`]): the reader builds its
/// table of objects and its page tree, and nothing on a page is read. For reading one object that
/// is checked on its own ([`illustrator_data_without_pdf_part`]), never for importing a page.
fn open_structural_with_options(bytes: &[u8], password: Option<&str>, options: &craft::pdf::OpenOptions) -> Result<Pdf, PdfError> {
    #[cfg(any(test, feature = "read-count"))]
    if let Ok(mut reads) = READS.lock() {
        let n = reads.entry(bytes.as_ptr() as usize).or_default();
        *n = n.saturating_add(1);
    }
    if is_postscript(bytes) {
        return Err(PdfError::PostScript);
    }
    if crate::streams::unsafe_file_streams(bytes) {
        return Err(PdfError::Parse(STREAM_NOTE.into()));
    }
    let password = password.unwrap_or_default();
    let read = |pw: &str| craft::pdf::open_slice_with_options(bytes, Some(pw), options);
    read(password)
        .or_else(|e| match e {
            // The reader takes only the open password of RC4 and 128-bit AES files: the
            // permissions password gives it.
            craft::pdf::OpenError::Load(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) if !password.is_empty() => {
                crate::encrypt::user_password(bytes, password).map_or(Err(e), |user| read(&user))
            }
            e => Err(e),
        })
        .map_err(|e| match e {
            craft::pdf::OpenError::Load(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) if password.is_empty() => {
                PdfError::NeedsPassword
            }
            craft::pdf::OpenError::Load(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => PdfError::WrongPassword,
            e => PdfError::Parse(e.to_string()),
        })
}

/// The pages and their boxes. An encrypted PDF needs its `password`
/// ([`PdfError::NeedsPassword`] / [`PdfError::WrongPassword`]).
pub fn info(bytes: &[u8], password: Option<&str>) -> Result<PdfInfo, PdfError> {
    Ok(info_in(&open(bytes, password)?))
}

/// [`info`] of a parsed PDF.
fn info_in(pdf: &Pdf) -> PdfInfo {
    let pages = pdf
        .pages()
        .iter()
        .map(|page| {
            let (w, h) = page.render_dimensions();
            let rotation = match page.rotation() {
                craft::pdf::syntax::page::Rotation::None => 0,
                craft::pdf::syntax::page::Rotation::Horizontal => 90,
                craft::pdf::syntax::page::Rotation::Flipped => 180,
                craft::pdf::syntax::page::Rotation::FlippedHorizontal => 270,
            };
            let unit = f64::from(page.user_unit());
            let boxes = CropTo::ALL.iter().filter(|c| **c != CropTo::Bounding).map(|c| (*c, page_box(page, *c).scale_from_origin(unit))).collect();
            PageInfo { width: w as f64, height: h as f64, rotation, boxes }
        })
        .collect();
    PdfInfo { pages }
}

/// The 0-based pages `opts` pick out of `count`.
pub(crate) fn picked(opts: &ImportOptions, count: usize) -> Result<Vec<usize>, PdfError> {
    let mut v = match &opts.pages {
        Some(p) => {
            if let Some(bad) = p.iter().find(|i| **i >= count) {
                return Err(PdfError::BadPage(bad.saturating_add(1), count));
            }
            p.clone()
        }
        None => (0..count).collect(),
    };
    if let Some(m) = opts.max_pages {
        v.truncate(m);
    }
    if v.is_empty() {
        return Err(PdfError::NoPages);
    }
    Ok(v)
}

fn rect(r: craft::pdf::syntax::object::Rect) -> Rect {
    Rect::new(r.x0, r.y0, r.x1, r.y1).abs()
}

/// Box `which` of `page` in PDF user space, clipped to the media box (Bounding: the crop box,
/// which the art's bounds replace after import). A missing or empty box is the crop box.
pub(crate) fn page_box(page: &Page<'_>, which: CropTo) -> Rect {
    let media = rect(page.media_box());
    let crop = rect(page.intersected_crop_box());
    let key: &[u8] = match which {
        CropTo::Bounding | CropTo::Crop => return crop,
        CropTo::Media if media.area() > 0.0 => return media,
        CropTo::Media => return crop,
        CropTo::Bleed => b"BleedBox",
        CropTo::Trim => b"TrimBox",
        CropTo::Art => b"ArtBox",
    };
    page.raw().get::<craft::pdf::syntax::object::Rect>(key).map(|r| rect(r).intersect(media)).filter(|r| r.area() > 0.0).unwrap_or(crop)
}

/// Where `page` draws on its artboard: the page transform (PDF user space → y-down page space
/// with the crop box at the origin, rotation applied) and `which` box in that space (a page with
/// no area gets the size viewers give it).
pub(crate) fn frame(page: &Page<'_>, which: CropTo) -> (Affine, Rect) {
    let init = Affine::new(page.initial_transform(true).as_coeffs());
    let b = init.transform_rect_bbox(page_box(page, which));
    let (w, h) = page.render_dimensions();
    (init, if b.is_finite() && b.area() > 0.0 { b } else { Rect::new(0.0, 0.0, w as f64, h as f64) })
}

/// How far `page`'s bleed box reaches past its trim box as the page shows (rotated): the bleed
/// `[top, bottom, left, right]` in points, kept to 0–[`MAX_BLEED`].
pub(crate) fn bleed(page: &Page<'_>) -> [f64; 4] {
    let init = Affine::new(page.initial_transform(true).as_coeffs());
    let (trim, bleed) = (init.transform_rect_bbox(page_box(page, CropTo::Trim)), init.transform_rect_bbox(page_box(page, CropTo::Bleed)));
    [trim.y0 - bleed.y0, bleed.y1 - trim.y1, trim.x0 - bleed.x0, bleed.x1 - trim.x1]
        .map(|b| if b.is_finite() { b.clamp(0.0, MAX_BLEED) } else { 0.0 })
}

/// The `PieceInfo` key such files keep their private data under.
const PRIVATE_DATA_OWNER: &[u8] = b"Illustrator"; // brand-ok: the key the files use

/// Most bytes of an editor's private data read.
const MAX_PRIVATE: usize = 256 << 20;

/// The private data of an Illustrator `.ai` (its first page's `PieceInfo`): the `AIPrivateData`
/// streams in order, joined. A PDF saved with the editing data has its streams under
/// `AIPDFPrivateData1`, `AIPDFPrivateData2`, … instead: those are read when there is no
/// `AIPrivateData1`. `None` for a PDF without either (or with a stream that can't be read).
/// What it holds is the editor's own copy of the art (see `vectorcraft_eps::layered_ai`).
pub fn illustrator_data(bytes: &[u8], password: Option<&str>) -> Option<Vec<u8>> {
    private_data(&open(bytes, password).ok()?)
}

/// The private data of a file whose PDF part can't be read ([`illustrator_data_without_pdf_part`]),
/// with the size of its first page.
#[derive(Clone, Debug, PartialEq)]
pub struct IllustratorData<T = Vec<u8>> {
    /// The private data, as [`illustrator_data`] reads it, or what was read from it
    /// ([`read_illustrator_data_without_pdf_part_with`]).
    pub data: T,
    /// The size in points of a unit of the first page's user space (its `UserUnit`).
    pub user_unit: f64,
    /// The first page's trim box (its crop box when it has none) in points, rotated as the page
    /// shows: width and height.
    pub trim: (f64, f64),
}

/// [`illustrator_data`] for a file whose PDF part can't be read (a stream the import refuses, or
/// objects built on one another without end): the file is opened without looking at its other
/// objects, and only the private data streams are read, with the same limits. `None` when the
/// file itself can't be opened, or as for [`illustrator_data`].
pub fn illustrator_data_without_pdf_part(bytes: &[u8], password: Option<&str>) -> Option<IllustratorData> {
    read_illustrator_data_without_pdf_part(bytes, password).ok().flatten()
}

/// [`illustrator_data_without_pdf_part`], with why the private data can't be read: `Ok(None)`
/// when the file can't be opened or has no private data, `Err` (for a note, such as "it is larger
/// than 256 MB") when it has private data that can't be read.
pub fn read_illustrator_data_without_pdf_part(bytes: &[u8], password: Option<&str>) -> Result<Option<IllustratorData>, String> {
    let Ok(pdf) = open_structural_with_options(bytes, password, &open_options()) else { return Ok(None) };
    let Some(data) = read_private_data(&pdf)? else { return Ok(None) };
    Ok(with_first_page(&pdf, data))
}

/// [`read_illustrator_data_without_pdf_part`] with the private data given to `read` as
/// [`Opened::read_illustrator_data`] gives it, in place of joined: what `read` returns, with the
/// size of the first page. `Err` with why a stream can't be read when one can't, even after `read`
/// has started.
pub fn read_illustrator_data_without_pdf_part_with<T>(
    bytes: &[u8],
    password: Option<&str>,
    read: impl FnOnce(&mut dyn std::io::Read) -> T,
) -> Result<Option<IllustratorData<T>>, String> {
    let Ok(pdf) = open_structural_with_options(bytes, password, &open_options()) else { return Ok(None) };
    let Some(data) = streamed_private_data(&pdf, read)? else { return Ok(None) };
    Ok(with_first_page(&pdf, data))
}

/// `data` read from `pdf`'s private data, with the size of its first page: `None` without a page.
fn with_first_page<T>(pdf: &Pdf, data: T) -> Option<IllustratorData<T>> {
    let page = pdf.pages().first()?;
    // (The page transform is in points: it applies the `UserUnit`.)
    let trim = frame(page, CropTo::Trim).1;
    Some(IllustratorData { data, user_unit: f64::from(page.user_unit()), trim: (trim.width(), trim.height()) })
}

/// The private data of `pdf`'s first page, as [`illustrator_data`] reads it.
fn private_data(pdf: &Pdf) -> Option<Vec<u8>> {
    read_private_data(pdf).ok().flatten()
}

/// The dictionary that holds `pdf`'s private data streams (its first page's `PieceInfo`), with the
/// key they are numbered after: `AIPrivateData`, else `AIPDFPrivateData`.
fn private_streams<'a>(pdf: &'a Pdf) -> Option<(Dict<'a>, &'static str)> {
    let private = pdf.pages().first()?.raw().get::<Dict<'_>>(b"PieceInfo")?.get::<Dict<'_>>(PRIVATE_DATA_OWNER)?.get::<Dict<'_>>(b"Private")?;
    let key = if private.contains_key(b"AIPrivateData1") { "AIPrivateData" } else { "AIPDFPrivateData" };
    Some((private, key))
}

/// Most private data streams read.
const MAX_PRIVATE_STREAMS: u32 = 100_000;

/// [`private_data`], with why a private data stream can't be read ([`inflated`]).
fn read_private_data(pdf: &Pdf) -> Result<Option<Vec<u8>>, String> {
    let Some((private, key)) = private_streams(pdf) else { return Ok(None) };
    let mut data = vec![];
    for n in 1..=MAX_PRIVATE_STREAMS {
        let Some(stream) = private.get::<Stream<'_>>(format!("{key}{n}").as_bytes()) else { break };
        data.extend_from_slice(&inflated(&stream, MAX_PRIVATE - data.len()).map_err(|why| why.to_string())?);
    }
    Ok((!data.is_empty()).then_some(data))
}

/// What `read` returns, given the private data of `pdf`'s first page as one reader over its
/// streams in order ([`PrivateStreams`]): `Ok(None)` without private data or when its streams hold
/// nothing, as [`read_private_data`] gives, and `Err` with why a stream can't be read when one
/// can't (in place of what `read` returned).
fn streamed_private_data<T>(pdf: &Pdf, read: impl FnOnce(&mut dyn std::io::Read) -> T) -> Result<Option<T>, String> {
    use std::io::BufRead as _;
    let Some((private, key)) = private_streams(pdf) else { return Ok(None) };
    let mut streams = PrivateStreams { private, key, next: 1, current: None, error: None };
    let mut src = std::io::BufReader::new(&mut streams);
    let read = match src.fill_buf() {
        Ok(first) if !first.is_empty() => Some(read(&mut src)),
        _ => None,
    };
    drop(src);
    match streams.error {
        Some(why) => Err(why),
        None => Ok(read),
    }
}

/// A reader over a `.ai`'s private data streams in order (`{key}1`, `{key}2`, … in `private`, as
/// [`read_private_data`] takes them), each checked ([`stream_form`]) and decoded as it is reached.
/// Once a stream can't be read, `error` says why and every read fails.
struct PrivateStreams<'a> {
    private: Dict<'a>,
    key: &'static str,
    /// The number of the next stream.
    next: u32,
    /// The stream being read, decoded.
    current: Option<Box<dyn std::io::Read + 'a>>,
    error: Option<String>,
}

impl std::io::Read for PrivateStreams<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(why) = &self.error {
                return Err(std::io::Error::other(why.clone()));
            }
            if let Some(current) = &mut self.current {
                match current.read(buf) {
                    Ok(0) => self.current = None,
                    Ok(n) => return Ok(n),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    // (Only a deflated stream's decoder fails.)
                    Err(_) => self.error = Some("its zlib compression is damaged".to_string()),
                }
                continue;
            }
            if self.next > MAX_PRIVATE_STREAMS {
                return Ok(0);
            }
            let Some(stream) = self.private.get::<Stream<'_>>(format!("{}{}", self.key, self.next).as_bytes()) else { return Ok(0) };
            self.next += 1;
            match stream_form(&stream) {
                Ok(true) => self.current = Some(Box::new(flate2::read::ZlibDecoder::new(std::io::Cursor::new(stream.raw_data())))),
                Ok(false) => self.current = Some(Box::new(std::io::Cursor::new(stream.raw_data()))),
                Err(why) => self.error = Some(why),
            }
        }
    }
}

/// Does the PDF's first page carry an editor's copy of the art: a PDF-compatible `.ai`
/// (`AIPrivateData` keys) or a PDF saved with its editing data (`AIPDFPrivateData`, #1345)? Without
/// reading the data. Either's trim boxes are its editor's artboards.
pub fn has_illustrator_data(bytes: &[u8], password: Option<&str>) -> bool {
    open(bytes, password).is_ok_and(|pdf| has_private_data_keys(&pdf))
}

/// [`has_illustrator_data`] of a parsed PDF.
fn has_private_data_keys(pdf: &Pdf) -> bool {
    pdf.pages().first().is_some_and(|p| piece_info_has(p, &[b"AIPrivateData", b"AIPDFPrivateData"]))
}

/// `stream` decoded, up to `most` bytes: the decoder is run here rather than by the PDF reader's
/// `decoded()`, which has no limit, so a stream of zeros can't grow to the size of the memory. Only a
/// stream that is plain or only deflated is read (the private data is no other), without decode
/// parameters; else, or when it is larger once decoded, `Err` with why. A filter the reader
/// doesn't know counts as another filter.
fn inflated(stream: &Stream<'_>, most: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let too_large = || format!("it is larger than {} MB", MAX_PRIVATE >> 20);
    let deflated = stream_form(stream)?;
    let raw = stream.raw_data();
    let mut out = vec![];
    let limit = u64::try_from(most).ok().and_then(|m| m.checked_add(1)).ok_or_else(too_large)?;
    if deflated {
        flate2::read::ZlibDecoder::new(&*raw).take(limit).read_to_end(&mut out).map_err(|_| "its zlib compression is damaged".to_string())?;
    } else if raw.len() <= most {
        out.extend_from_slice(&raw);
    } else {
        return Err(too_large());
    }
    if out.len() > most {
        return Err(too_large());
    }
    Ok(out)
}

/// Is the private data stream `stream` deflated (`true`) or plain (`false`)? `Err` with why it
/// isn't read when it is in another form: with decode parameters, or with a filter the reader
/// doesn't know (which counts as another filter).
fn stream_form(stream: &Stream<'_>) -> Result<bool, String> {
    let unread = || "it is stored in a form that isn't read".to_string();
    let dict = stream.dict();
    let named = |key: &[u8]| match dict.get::<Object<'_>>(key) {
        None | Some(Object::Null(_)) => Some(0),
        Some(Object::Name(_)) => Some(1),
        Some(Object::Array(a)) => Some(items(&a).count()),
        Some(_) => None,
    };
    let params = |key: &[u8]| match dict.get::<Object<'_>>(key) {
        None | Some(Object::Null(_)) => false,
        Some(Object::Array(a)) => items(&a).any(|m| !matches!(m, MaybeRef::NotRef(Object::Null(_)))),
        Some(_) => true,
    };
    if params(b"DecodeParms") || params(b"DP") {
        return Err(unread());
    }
    let filters = stream.filters();
    // (The reader leaves out the filters it doesn't know.)
    if named(b"Filter").zip(named(b"F")).is_none_or(|(a, b)| a.max(b) != filters.len()) {
        return Err(unread());
    }
    match filters.as_slice() {
        [] => Ok(false),
        [craft::pdf::syntax::Filter::FlateDecode] => Ok(true),
        _ => Err(unread()),
    }
}

/// Does `page` carry an editor's private data (`PieceInfo` keys starting with `AIPrivateData`)?
/// A file saved without PDF compatibility keeps its art there and shows a placeholder page.
pub(crate) fn has_private_data(page: &Page<'_>) -> bool {
    piece_info_has(page, &[b"AIPrivateData"])
}

/// Does `page` state where its art is: an art, trim or bleed box (ISO 32000-1 §14.11.2, the
/// extent of the page's meaningful content and of the finished page)? A page written from an
/// artboard has them; the notice a file saved without its PDF part shows instead of its art is
/// the same page whatever the document, with no art to state the place of.
pub(crate) fn states_art(page: &Page<'_>) -> bool {
    [&b"ArtBox"[..], b"TrimBox", b"BleedBox"].iter().any(|k| page.raw().get::<[f32; 4]>(*k).is_some())
}

/// Does `page`'s `PieceInfo` hold a key starting with one of `prefixes`, up to four dictionaries in?
fn piece_info_has(page: &Page<'_>, prefixes: &[&[u8]]) -> bool {
    fn walk(d: &Dict<'_>, prefixes: &[&[u8]], depth: u32) -> bool {
        depth < 4
            && d.keys().any(|k| {
                prefixes.iter().any(|p| k.starts_with(p)) || d.get::<Dict<'_>>(k.as_ref()).is_some_and(|sub| walk(&sub, prefixes, depth + 1))
            })
    }
    page.raw().get::<Dict<'_>>(b"PieceInfo").is_some_and(|d| walk(&d, prefixes, 0))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decoded_sizes_read_in_whole_units() {
        assert_eq!(super::decoded_size(256 << 20), "256 MB");
        assert_eq!(super::decoded_size(418 << 20), "418 MB");
        assert_eq!(super::decoded_size(1 << 30), "1 GB");
        assert_eq!(super::decoded_size((1 << 30) + (1 << 20)), "1025 MB");
    }
}
