use super::writer::{Sheet, Writer, date_time};
use super::{Affine, Document, ExportReport, Exporter, HashSet, PageBoxes, PdfError, PdfOptions, Rect, krilla, write};
use crate::{Compatibility, PdfSettings, Standard};

/// Export `doc` as PDF bytes: one page per artboard (or the artboards chosen in `opts`).
pub fn export(doc: &Document, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    export_with_report(doc, opts).map(|r| r.bytes)
}

/// Like [`export`], also returning warnings, features approximated or dropped, and notes on the
/// options: options accepted but not applied yet (see [`crate::PdfSettings::warnings`]) or applied
/// another way.
pub fn export_with_report(doc: &Document, opts: &PdfOptions) -> Result<ExportReport, PdfError> {
    // Type in the document's composite fonts is drawn in them, whether or not an app has the document open.
    let _lent = vectorcraft_text::FontDb::global().lend_composite_fonts(&doc.composite_fonts);
    write::guard(|| leaving_out_undecodable(doc, |left_out| export_with_report_inner(doc, opts, left_out)))
}

/// Run `write` (a whole PDF written by a [`Writer`]), and run it again each time it fails on an
/// image whose pixel data the writer couldn't decode, with that image left out: [`Writer::finish`]
/// adds the image's key to the set `write` is given, and the exporters leave out the images it
/// holds. The writer reports such an image (a PNG, GIF or WebP kept as it is, whose header reads)
/// only when it finishes the file. Each repeated run leaves out at least one more image.
pub(crate) fn leaving_out_undecodable<T>(doc: &Document, mut write: impl FnMut(&mut HashSet<String>) -> Result<T, PdfError>) -> Result<T, PdfError> {
    // The most runs repeated: each is a whole export, and a document of many images that don't
    // decode would make them quadratic. Past it, one last run leaves out every image of `doc`, so
    // the export still finishes (the images left out are reported).
    const MAX_RETRIES: usize = 16;
    let mut left_out = HashSet::new();
    let mut retries = 0;
    loop {
        let known = left_out.len();
        match write(&mut left_out) {
            Err(_) if left_out.len() > known && retries < MAX_RETRIES => retries += 1,
            Err(_) if left_out.len() > known && retries == MAX_RETRIES => {
                left_out.extend(doc.images.keys().cloned());
                retries += 1;
            }
            done => return done,
        }
    }
}

fn export_with_report_inner(doc: &Document, opts: &PdfOptions, left_out: &mut HashSet<String>) -> Result<ExportReport, PdfError> {
    let set = &opts.settings;
    set.check()?;
    vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
    // Live geometry effects export as their result, in symbols' art too; raster effects are
    // reported below.
    // Only the layers this export writes place their instances' art (see `ex.non_printing` and
    // `ex.layers` below).
    let layers = vectorcraft_effects::WrittenLayers {
        hidden: set.writes_layers() && !doc.page_knockout,
        non_printing: set.include_non_printing || set.create_layers,
    };
    let baked = vectorcraft_effects::bake_document_placing_symbols(doc, true, layers);
    // Baking stops early when cancelled, leaving a half-baked copy that must not be written.
    vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
    let doc = baked.as_ref().unwrap_or(doc);
    let pages = pages(doc, opts)?;
    let raised = for_long_pages(set, &pages)?;
    let set = raised.as_ref().unwrap_or(set);

    // The file's Title is File Info's; the page information names the document, as its tab does.
    let title = crate::pdfx::title(set.standard, opts.title.clone().unwrap_or_else(|| doc.info_title().to_owned()));
    let name = opts.title.as_deref().unwrap_or(&doc.title);
    // The time of writing (none on the web) dates the editing data and the page information.
    let now = vectorcraft_doc::metadata::output_time();
    let created_at = opts.created.or(now);
    crate::pdfx::require_date(set.standard, created_at)?;
    let display_doc_title = opts.display_doc_title || opts.title.as_deref().is_some_and(|title| !title.trim().is_empty());
    let mut w = Writer::new(doc, set, &title, created_at, doc.color_mode == vectorcraft_doc::ColorMode::Cmyk)?;
    // Preserve Editing: the native document as an embedded file (`check` refused PDF/A with it).
    let mut warnings = vec![];
    let mut settings = set.warnings();
    if raised.is_some() {
        settings.push(if opts.settings.compatibility == Compatibility::Pdf14 {
            "PDF 1.4 has no UserUnit, which a page longer than 14,400 pt (200 in) needs to stay within PDF/A-2's page limit of 14,400 units: the file is written as PDF 1.6".into()
        } else {
            "PDF 1.5 has no UserUnit, which a page longer than 14,400 pt (200 in) needs to keep its size: the file is written as PDF 1.6".into()
        });
    }
    let native = opts.native.as_deref().filter(|_| set.preserve_editing);
    if let Some(native) = native {
        w.pdf.document_mut().embed_file(crate::editing::embedded_file(native, set.compression.compress_text, now.map(date_time)));
    } else if set.preserve_editing {
        settings.push("Preserve editing needs the native document, which wasn't given: the PDF reopens as plain artwork".into());
    }

    let mut ex = Exporter::new(doc, set);
    ex.non_printing = set.include_non_printing || set.create_layers;
    ex.out = w.out.clone();
    ex.left_out = left_out.clone();
    // A knockout page group draws the layers' objects one by one, not layer by layer.
    ex.layers = set.writes_layers() && !doc.page_knockout;
    if set.writes_layers() && doc.page_knockout {
        settings.push("PDF layers can't be written with a knockout page group: every layer is plain page content".into());
    }
    ex.overprint = set.advanced.overprint == crate::Overprint::Preserve;
    let bleed = set.bleed_of(doc);
    let marks = set.marks.printer_marks();
    let scales = writes_user_units(set);
    // PDF/X files have no annotations over the art (ISO 15930: only printer's mark and trap
    // network annotations, outside the bleed box): objects' URLs aren't written as links there.
    let links = !set.standard.is_pdfx();
    let mut linked = false;
    for layer in doc.layers.iter().filter(|_| !links) {
        layer.walk(&mut |n| linked |= !n.is_ruler_guide() && n.url().is_some());
    }
    if linked {
        warnings.push(format!("{} files have no links: the objects' URLs aren't clickable", set.standard.label()));
    }
    let mut units = Vec::with_capacity(pages.len());
    let total = pages.len() as u64;
    let long: Vec<(usize, f64)> =
        pages.iter().map(|(i, b)| (*i, b.media.width().max(b.media.height()))).filter(|(_, side)| *side > MAX_PDF14_REAL).collect();
    for (written, (i, boxes)) in pages.into_iter().enumerate() {
        // The page count and the page being written, for a client that asked for progress.
        vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
        vectorcraft_doc::cancel::progress(written as u64 + 1, total, &format!("Writing page {} of {total}", written + 1));
        let (m, r) = (boxes.media, boxes.bleed);
        let info = if marks.page_info { crate::marks::page_info(doc, name, i, now) } else { String::new() };
        let art = crate::marks::art(doc, &marks, &boxes, bleed, &info);
        // The page is the media box, y down from its top-left corner (like ours), in units of
        // `unit` points.
        let unit = if scales { user_unit(m.width(), m.height()) } else { 1.0 };
        units.push(unit);
        if m.width().max(m.height()) > MAX_PAGE_SIDE {
            warnings.push(long_page_warning(set, i, m.width(), m.height(), unit, scales));
        }
        let shift = Affine::translate((-m.x0, -m.y0));
        let view = if unit > 1.0 { Affine::scale(1.0 / unit) * shift } else { shift };
        let sheet = Sheet {
            size: (m.width() / unit, m.height() / unit),
            unit,
            trim: view.transform_rect_bbox(boxes.trim),
            bleed: view.transform_rect_bbox(r),
            view,
            window: None,
            place: Affine::IDENTITY,
            copies: &[],
            area: r,
            // Art reaches as far as the bleed: the marks lie outside it.
            clip: m != r,
            marks: art.as_ref(),
            negative: false,
            links,
        };
        w.page(&mut ex, &sheet).map_err(|_| PdfError::BadArtboard(i))?;
    }
    vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
    let (layers, overprinted, invisible) = (ex.layers, ex.overprinted, ex.invisible);
    w.absorb(ex);
    let (bytes, more, catalog) = match w.finish(left_out, display_doc_title) {
        Ok(done) => done,
        Err(e) => return Err(writer_error(doc, opts, left_out, &long, e)),
    };
    let bytes = crate::forms::finish(bytes, doc, set, layers, overprinted, invisible)?;
    // The writer gives the file its creation date as its modification date too.
    let bytes = match opts.modified.filter(|m| Some(*m) != created_at) {
        Some(modified) => crate::post::modification_date(bytes, modified)?,
        None => bytes,
    };
    // The writer's metadata has no copyright: it is added to the XMP packet.
    let bytes = crate::post::rights(bytes, &doc.metadata)?;
    warnings.extend(more);
    settings.extend(catalog);
    let bytes = crate::post::user_units(bytes, &units)?;
    // A PDF 1.5 file written as PDF 1.6 keeps the encryption PDF 1.5 has (128-bit RC4).
    let bytes = crate::post::finish(bytes, opts, native.is_some(), &mut settings)?;
    warnings.dedup();
    settings.dedup();
    Ok(ExportReport { bytes, warnings, setting_notes: settings })
}

/// The artboards `opts` exports, in page order, with the boxes of their pages: Marks and Bleeds
/// make each page its artboard (the trim box) grown by the bleed, and by the printer's marks
/// around that.
fn pages(doc: &Document, opts: &PdfOptions) -> Result<Vec<(usize, PageBoxes)>, PdfError> {
    let indices: Vec<usize> = match &opts.artboards {
        Some(v) => v.clone(),
        None => (0..doc.artboards.len()).collect(),
    };
    if doc.artboards.is_empty() || indices.is_empty() {
        return Err(PdfError::NoArtboards);
    }
    let (bleed, marks) = (opts.settings.bleed_of(doc), opts.settings.marks.printer_marks());
    indices.into_iter().map(|i| Ok((i, PageBoxes::new(doc.artboards.get(i).ok_or(PdfError::BadArtboard(i))?.rect, bleed, &marks)))).collect()
}

/// What each page of `doc` exported with `opts` shows, in document space (its media box: the
/// artboard with the bleed and the printer's marks), in page order.
pub fn page_areas(doc: &Document, opts: &PdfOptions) -> Result<Vec<Rect>, PdfError> {
    Ok(pages(doc, opts)?.into_iter().map(|(_, b)| b.media).collect())
}

/// The largest side of a page in units of its default user space that ISO 32000-1 (Annex C.2)
/// gives: 14,400 (200 inches in units of 1/72 inch).
pub(crate) const MAX_PAGE_SIDE: f64 = 14_400.0;

/// The smallest side of a page in units of its default user space that ISO 32000-1 (Annex C.2)
/// gives: 3.
const MIN_PAGE_SIDE: f64 = 3.0;

/// The largest real number the writer puts in a PDF 1.3 or 1.4 file: past it, krilla fails the
/// file with `Limit(TooLargeFloat)`.
const MAX_PDF14_REAL: f64 = 32_767.0;

/// Pages with a side longer than [`MAX_PAGE_SIDE`] are written with a `UserUnit`: in PDF 1.6 and
/// later files that aren't PDF/X files, unless `long_pages_in_points` applies. Other files have
/// every page in points.
fn writes_user_units(set: &PdfSettings) -> bool {
    set.compatibility.has_user_unit() && !set.standard.is_pdfx() && !long_pages_in_points(set)
}

/// `long_pages_in_points` is on in a file without a standard. PDF/X files write long pages in
/// points anyway, and PDF/A files keep the `UserUnit`, which keeps their page boundaries within
/// 14,400 units (ISO 19005-2, 6.1.13). A PDF/A file at PDF 1.4 is written as PDF 1.6 to have one
/// ([`for_long_pages`]).
fn long_pages_in_points(set: &PdfSettings) -> bool {
    set.long_pages_in_points && set.standard == Standard::None
}

/// The `UserUnit` (PDF 1.6) of a page `w` × `h` points large: 1 when both sides are at most
/// [`MAX_PAGE_SIDE`], else the longer side divided by it, rounded up to the next thousandth, so
/// that both sides are at most [`MAX_PAGE_SIDE`] units. A short side of [`MIN_PAGE_SIDE`] points
/// or more stays at least [`MIN_PAGE_SIDE`] units: the unit is at most the short side divided by
/// it, rounded down to the next thousandth, and the long side is then longer than
/// [`MAX_PAGE_SIDE`] units.
fn user_unit(w: f64, h: f64) -> f64 {
    let (side, short) = (w.abs().max(h.abs()), w.abs().min(h.abs()));
    if !(side.is_finite() && side > MAX_PAGE_SIDE) {
        return 1.0;
    }
    let unit = (side / MAX_PAGE_SIDE * 1000.0).ceil() / 1000.0;
    if short >= MIN_PAGE_SIDE { unit.min((short / MIN_PAGE_SIDE * 1000.0).floor() / 1000.0).max(1.0) } else { unit }
}

/// Check the page sizes of `doc` exported with `opts` before anything is drawn: the error
/// [`export`] gives for a page that the file's PDF version or standard can't hold.
pub fn check_page_sizes(doc: &Document, opts: &PdfOptions) -> Result<(), PdfError> {
    for_long_pages(&opts.settings, &pages(doc, opts)?).map(|_| ())
}

/// The settings `set` is written with when one of `pages` has a side longer than
/// [`MAX_PAGE_SIDE`]: a PDF 1.5 file that isn't a PDF/X file is written as PDF 1.6, which has a
/// page's `UserUnit`, unless `long_pages_in_points` applies (`None`: `set` as it is). Other files
/// without a `UserUnit` have the page in points. The writer puts a whole number as an integer and fails a PDF 1.3 or 1.4 file that has
/// a real number past [`MAX_PDF14_REAL`]: a page whose side is past it and not a whole number of
/// points is refused here, before anything is drawn ([`page_too_long`]).
fn for_long_pages(set: &PdfSettings, pages: &[(usize, PageBoxes)]) -> Result<Option<PdfSettings>, PdfError> {
    if !pages.iter().any(|(_, b)| b.media.width().max(b.media.height()) > MAX_PAGE_SIDE) {
        return Ok(None);
    }
    let raised = (set.compatibility == Compatibility::Pdf15 && !set.standard.is_pdfx() && !long_pages_in_points(set))
        .then(|| PdfSettings { compatibility: Compatibility::Pdf16, ..set.clone() });
    // PDF/A-2 limits page boundaries to 14,400 units (ISO 19005-2, 6.1.13), and PDF 1.4 has no
    // `UserUnit`: a PDF/A file at PDF 1.4 is written as PDF 1.6, with the `UserUnit` whether or
    // not `long_pages_in_points` is on.
    let raised = raised.or_else(|| {
        (set.standard == Standard::PdfA2b && set.compatibility == Compatibility::Pdf14).then(|| PdfSettings {
            compatibility: Compatibility::Pdf16,
            long_pages_in_points: false,
            ..set.clone()
        })
    });
    // The writer's test for a whole number, on the `f32` it writes.
    let real = |side: f64| side > MAX_PDF14_REAL && (side as f32) as i32 as f32 != side as f32;
    if let Some((i, b)) = pages.iter().find(|(_, b)| real(b.media.width()) || real(b.media.height())) {
        page_too_long(raised.as_ref().unwrap_or(set), *i, b.media.width().max(b.media.height()))?;
    }
    Ok(raised)
}

/// The error for the page of artboard `artboard` (0-based), `side` points long, whose size or art
/// needs a real number past [`MAX_PDF14_REAL`], in a PDF 1.3 or 1.4 file written with `set`; `Ok`
/// in other files.
fn page_too_long(set: &PdfSettings, artboard: usize, side: f64) -> Result<(), PdfError> {
    if !matches!(set.compatibility, Compatibility::Pdf13 | Compatibility::Pdf14) {
        return Ok(());
    }
    // PDF/X-1a and PDF/X-3 files are always PDF 1.3.
    let label = if matches!(set.standard, Standard::PdfX1a | Standard::PdfX3) { set.standard.label() } else { set.compatibility.label() };
    let (max, n, side) = (thousands(MAX_PDF14_REAL), artboard + 1, format!("{:.0}", side.ceil()));
    Err(PdfError::BadSetting(format!(
        "{label} files hold real numbers up to {max}, and the page of artboard {n}, {side} pt long, needs larger ones: choose PDF 1.6 or later without a PDF/X standard, or a smaller artboard"
    )))
}

/// The writer's `error` for `doc` exported with `opts`, or, when it failed on a real number past
/// [`MAX_PDF14_REAL`] in a PDF 1.3 or 1.4 file, the error for the page of `long` (artboards and
/// sides past it) that needs one. With several such pages, the first [`MAX_PROBES`] are each
/// written alone, without the editing data and the thumbnails, to find it; past them, the first
/// page is named.
fn writer_error(doc: &Document, opts: &PdfOptions, left_out: &mut HashSet<String>, long: &[(usize, f64)], error: PdfError) -> PdfError {
    const MAX_PROBES: usize = 8;
    let too_large = format!("{:?}", krilla::error::KrillaError::Limit(krilla::error::LimitError::TooLargeFloat));
    if !matches!(&error, PdfError::Write(e) if *e == too_large) {
        return error;
    }
    let failing = match long {
        [] => return error,
        [one] => *one,
        [first, ..] => long
            .iter()
            .take(MAX_PROBES)
            .copied()
            .find(|(i, _)| {
                let alone = PdfOptions {
                    settings: opts.settings.clone(),
                    artboards: Some(vec![*i]),
                    title: opts.title.clone(),
                    display_doc_title: opts.display_doc_title,
                    created: opts.created,
                    modified: opts.modified,
                    native: None,
                    private_data: None,
                    thumbnails: vec![],
                };
                matches!(export_with_report_inner(doc, &alone, left_out), Err(PdfError::BadSetting(_)))
            })
            .unwrap_or(*first),
    };
    page_too_long(&opts.settings, failing.0, failing.1).err().unwrap_or(error)
}

/// `v` (a whole number) with commas between groups of three digits: 32,767.
fn thousands(v: f64) -> String {
    let digits = format!("{v:.0}");
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A page `w` × `h` points large in points, rounded up, and in meters: 56693 × 2835 pt (20.00 ×
/// 1.00 m).
fn page_size(w: f64, h: f64) -> String {
    const METERS_PER_POINT: f64 = 0.0254 / 72.0;
    format!("{:.0} × {:.0} pt ({:.2} × {:.2} m)", w.ceil(), h.ceil(), w * METERS_PER_POINT, h * METERS_PER_POINT)
}

/// The warning for the page of artboard `artboard` (0-based), `w` × `h` points large with a side
/// longer than [`MAX_PAGE_SIDE`], written in units of `unit` points; `scales`: the file has
/// `UserUnit`s.
fn long_page_warning(set: &PdfSettings, artboard: usize, w: f64, h: f64, unit: f64, scales: bool) -> String {
    let n = artboard + 1;
    let size = page_size(w, h);
    if scales && w.max(h) / unit > MAX_PAGE_SIDE {
        format!(
            "the page of artboard {n} is {size}, too long and narrow for PDF's page sides of 3 to 14,400 units: it is written with a UserUnit of {unit}, which keeps the short side at 3 units or more, and apps that keep to the limit cut the long side to 14,400 units"
        )
    } else if unit > 1.0 {
        format!(
            "the page of artboard {n} is {size}, longer than PDF's page limit of 14,400 pt (200 in): it is written with a UserUnit of {unit}, so apps that ignore UserUnit show it that many times smaller"
        )
    } else {
        let label = if set.standard.is_pdfx() || set.standard == Standard::PdfA2b { set.standard.label() } else { set.compatibility.label() };
        format!(
            "the page of artboard {n} is {size}, longer than PDF's page limit of 14,400 pt (200 in): {label} pages are written without a UserUnit, in points, and apps that keep to the limit cut the page to 200 in"
        )
    }
}

/// The RGB colours and non-CMYK images a CMYK document carries, which the export separates into
/// CMYK. Their darkest values come out as rich blacks, so the export says how many there are.
#[derive(Default)]
pub(crate) struct RgbInCmyk {
    /// Distinct RGB colours, by the bits of their channels.
    pub(super) colours: HashSet<[u32; 3]>,
    /// Keys of the raster images that aren't CMYK.
    pub(super) images: HashSet<String>,
}

impl RgbInCmyk {
    pub(super) fn merge(&mut self, other: RgbInCmyk) {
        self.colours.extend(other.colours);
        self.images.extend(other.images);
    }

    /// The warning for the export report; `None` when the document has nothing of the sort.
    pub(super) fn warning(&self) -> Option<String> {
        let (c, i) = (self.colours.len(), self.images.len());
        if c == 0 && i == 0 {
            return None;
        }
        let mut parts = vec![];
        if c > 0 {
            parts.push(format!("{c} RGB colour{}", if c == 1 { "" } else { "s" }));
        }
        if i > 0 {
            parts.push(format!("{i} RGB or grey image{}", if i == 1 { "" } else { "s" }));
        }
        Some(format!(
            "this CMYK document has {}: they are converted to CMYK through the colour settings, so dark ones can come out as rich blacks on several plates; paint them with CMYK colours (Edit › Edit Colors › Convert to CMYK) for pure black ink",
            parts.join(" and ")
        ))
    }
}
