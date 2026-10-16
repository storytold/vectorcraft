//! The pages of a PDF to import: opening with a password, the pages picked, the box each page
//! is cropped to ([`CropTo`]) and [`info`].

use std::collections::HashMap;

use craft_pdf::syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream, dict_or_stream};
use craft_pdf::syntax::page::Page;
use craft_pdf::syntax::xref::XRef;
use craft_pdf::syntax::{DecryptionError, LoadPdfError, Pdf};
use kurbo::{Affine, Rect};
use vectorcraft_doc::setup::MAX_BLEED;

use crate::import_color::{direct, items, readable, values};
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

/// The error of a file with a stream the reader would fail on ([`crate::streams`]).
const STREAM_NOTE: &str = "a stream in it would decode to more than 256 MB, or its decode parameters can't be applied";

/// The error of a file with functions or color spaces built on one another past [`MAX_LINKS`]
/// (or on themselves), or a calculator function nested past [`MAX_PROCEDURES`].
const NESTED_NOTE: &str = "its functions or colour spaces are built on one another without end";

/// The error of a file with an array whose items can't all be read (a damaged file).
const UNREADABLE_NOTE: &str = "an array in it is damaged and can't be read";

/// Why `pdf` is refused, if it is: a stream the reader would fail on ([`STREAM_NOTE`]), or
/// functions, color spaces or calculator procedures nested too deep ([`NESTED_NOTE`]). Every
/// object is looked at, and every array, dictionary and stream written inside one, wherever it
/// is used from: the interpreter builds functions and color spaces from resources, shadings,
/// images, patterns, masks and graphics states alike.
fn hostile(pdf: &Pdf) -> Option<&'static str> {
    let xref = pdf.xref();
    let mut known = HashMap::new();
    // The objects of the file, then what each holds directly (64 deep at most: the reader's cap).
    let mut pending: Vec<Object<'_>> = pdf.objects().into_iter().collect();
    while let Some(o) = pending.pop() {
        // A stream's own checks come first: a calculator function is decoded below.
        if matches!(&o, Object::Stream(s) if crate::streams::unsafe_stream(s)) {
            return Some(STREAM_NOTE);
        }
        if procedures_too_deep(&o) || built_on(MaybeRef::NotRef(o.clone()), xref, &mut known, MAX_LINKS).is_none() {
            return Some(NESTED_NOTE);
        }
        match &o {
            // An array with an item that can't be read can't be looked at whole, and the
            // interpreter would still read its leading items: the file is refused.
            Object::Array(a) if !readable(a) => return Some(UNREADABLE_NOTE),
            Object::Array(a) => pending.extend(items(a).filter_map(direct)),
            _ => pending.extend(dict_or_stream(&o).into_iter().flat_map(|(d, _)| values(d)).filter_map(direct)),
        }
    }
    None
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

/// Preserve the application's existing accepted file sizes. Ordinary stream decoding still
/// uses the shared reader's immutable per-document limits, and its input copy is fallible.
pub(crate) fn open_options() -> craft_pdf::OpenOptions {
    craft_pdf::OpenOptions { max_input_bytes: usize::MAX, ..Default::default() }
}

pub(crate) fn open_with_options(bytes: &[u8], password: Option<&str>, options: &craft_pdf::OpenOptions) -> Result<Pdf, PdfError> {
    if is_postscript(bytes) {
        return Err(PdfError::PostScript);
    }
    if crate::streams::unsafe_file_streams(bytes) {
        return Err(PdfError::Parse(STREAM_NOTE.into()));
    }
    let password = password.unwrap_or_default();
    let read = |pw: &str| craft_pdf::open_slice_with_options(bytes, Some(pw), options);
    read(password)
        .or_else(|e| match e {
            // The reader takes only the open password of RC4 and 128-bit AES files: the
            // permissions password gives it.
            craft_pdf::OpenError::Load(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) if !password.is_empty() => {
                crate::encrypt::user_password(bytes, password).map_or(Err(e), |user| read(&user))
            }
            e => Err(e),
        })
        .map_err(|e| match e {
            craft_pdf::OpenError::Load(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) if password.is_empty() => {
                PdfError::NeedsPassword
            }
            craft_pdf::OpenError::Load(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => PdfError::WrongPassword,
            e => PdfError::Parse(e.to_string()),
        })
        // Twice: looking at every object makes the reader repair a damaged table of objects (once
        // only), and the objects are other ones after that.
        .and_then(|pdf| match (0..2).find_map(|_| hostile(&pdf)) {
            Some(why) => Err(PdfError::Parse(why.into())),
            None => Ok(pdf),
        })
}

/// The pages and their boxes. An encrypted PDF needs its `password`
/// ([`PdfError::NeedsPassword`] / [`PdfError::WrongPassword`]).
pub fn info(bytes: &[u8], password: Option<&str>) -> Result<PdfInfo, PdfError> {
    let pdf = open(bytes, password)?;
    let pages = pdf
        .pages()
        .iter()
        .map(|page| {
            let (w, h) = page.render_dimensions();
            let rotation = match page.rotation() {
                craft_pdf::syntax::page::Rotation::None => 0,
                craft_pdf::syntax::page::Rotation::Horizontal => 90,
                craft_pdf::syntax::page::Rotation::Flipped => 180,
                craft_pdf::syntax::page::Rotation::FlippedHorizontal => 270,
            };
            let unit = f64::from(page.user_unit());
            let boxes = CropTo::ALL.iter().filter(|c| **c != CropTo::Bounding).map(|c| (*c, page_box(page, *c).scale_from_origin(unit))).collect();
            PageInfo { width: w as f64, height: h as f64, rotation, boxes }
        })
        .collect();
    Ok(PdfInfo { pages })
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

fn rect(r: craft_pdf::syntax::object::Rect) -> Rect {
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
    page.raw().get::<craft_pdf::syntax::object::Rect>(key).map(|r| rect(r).intersect(media)).filter(|r| r.area() > 0.0).unwrap_or(crop)
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
    let pdf = open(bytes, password).ok()?;
    let page = pdf.pages().first()?;
    let private = page.raw().get::<Dict<'_>>(b"PieceInfo")?.get::<Dict<'_>>(PRIVATE_DATA_OWNER)?.get::<Dict<'_>>(b"Private")?;
    let key = if private.contains_key(b"AIPrivateData1") { "AIPrivateData" } else { "AIPDFPrivateData" };
    let mut data = vec![];
    for n in 1..=100_000u32 {
        let Some(stream) = private.get::<Stream<'_>>(format!("{key}{n}").as_bytes()) else { break };
        data.extend_from_slice(&inflated(&stream, MAX_PRIVATE - data.len())?);
    }
    (!data.is_empty()).then_some(data)
}

/// Does the PDF's first page carry an editor's copy of the art: a PDF-compatible `.ai`
/// (`AIPrivateData` keys) or a PDF saved with its editing data (`AIPDFPrivateData`, #1345)? Without
/// reading the data. Either's trim boxes are its editor's artboards.
pub fn has_illustrator_data(bytes: &[u8], password: Option<&str>) -> bool {
    open(bytes, password).is_ok_and(|pdf| pdf.pages().first().is_some_and(|p| piece_info_has(p, &[b"AIPrivateData", b"AIPDFPrivateData"])))
}

/// `stream` decoded, up to `most` bytes: the decoder is run here rather than by the PDF reader's
/// `decoded()`, which has no limit, so a stream of zeros can't grow to the size of the memory. Only a
/// stream that is plain or only deflated is read (the private data is no other); else, or when it
/// is larger once decoded, `None`.
fn inflated(stream: &Stream<'_>, most: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    if stream.dict().get::<Dict<'_>>(b"DecodeParms").is_some() {
        return None;
    }
    let raw = stream.raw_data();
    let mut out = vec![];
    let limit = u64::try_from(most).ok()?.checked_add(1)?;
    match stream.filters().len() {
        0 => out.extend_from_slice(&raw),
        1 => {
            flate2::read::ZlibDecoder::new(&*raw).take(limit).read_to_end(&mut out).ok()?;
        }
        _ => return None,
    }
    (out.len() <= most).then_some(out)
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
