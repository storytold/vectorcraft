use super::writer::{Sheet, Writer, date_time};
use super::*;

/// Export `doc` as PDF bytes: one page per artboard (or the artboards chosen in `opts`).
pub fn export(doc: &Document, opts: &PdfOptions) -> Result<Vec<u8>, PdfError> {
    export_with_report(doc, opts).map(|r| r.bytes)
}

/// Like [`export`], also returning warnings: options accepted but not applied yet (see
/// [`crate::PdfSettings::warnings`]) and features approximated or dropped.
pub fn export_with_report(doc: &Document, opts: &PdfOptions) -> Result<ExportReport, PdfError> {
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
    // Live geometry effects export as their result; raster effects are reported below.
    let baked = vectorcraft_effects::bake_document(doc);
    // Baking stops early when cancelled, leaving a half-baked copy that must not be written.
    vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
    let doc = baked.as_ref().unwrap_or(doc);
    let pages = pages(doc, opts)?;

    let title = crate::pdfx::title(set.standard, opts.title.clone().unwrap_or_else(|| doc.title.clone()));
    // The time of writing (none on the web) dates the editing data and the page information.
    let now = vectorcraft_doc::metadata::output_time();
    let created_at = opts.created.or(now);
    crate::pdfx::require_date(set.standard, created_at)?;
    let display_doc_title = opts.display_doc_title || opts.title.as_deref().is_some_and(|title| !title.trim().is_empty());
    let mut w = Writer::new(doc, set, &title, created_at, doc.color_mode == vectorcraft_doc::ColorMode::Cmyk)?;
    // Preserve Editing: the native document as an embedded file (`check` refused PDF/A with it).
    let mut warnings = set.warnings();
    let native = opts.native.as_deref().filter(|_| set.preserve_editing);
    if let Some(native) = native {
        w.pdf.document_mut().embed_file(crate::editing::embedded_file(native, set.compression.compress_text, now.map(date_time)));
    } else if set.preserve_editing {
        warnings.push("Preserve editing needs the native document, which wasn't given: the PDF reopens as plain artwork".into());
    }

    let mut ex = Exporter::new(doc, set);
    ex.non_printing = set.include_non_printing || set.create_layers;
    ex.out = w.out.clone();
    ex.left_out = left_out.clone();
    // A knockout page group draws the layers' objects one by one, not layer by layer.
    ex.layers = set.writes_layers() && !doc.page_knockout;
    if set.writes_layers() && doc.page_knockout {
        warnings.push("PDF layers can't be written with a knockout page group: every layer is plain page content".into());
    }
    ex.overprint = set.advanced.overprint == crate::Overprint::Preserve;
    let bleed = set.bleed_of(doc);
    let marks = set.marks.printer_marks();
    // In PDF 1.6 and later files that aren't PDF/X files, a page with a side longer than
    // [`MAX_PAGE_SIDE`] is written in larger units; other files have every page in points.
    let scales = set.compatibility.has_user_unit() && !set.standard.is_pdfx();
    let mut units = Vec::with_capacity(pages.len());
    let total = pages.len() as u64;
    for (written, (i, boxes)) in pages.into_iter().enumerate() {
        // The page count and the page being written, for a client that asked for progress.
        vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
        vectorcraft_doc::cancel::progress(written as u64 + 1, total, &format!("Writing page {} of {total}", written + 1));
        let (m, r) = (boxes.media, boxes.bleed);
        let info = if marks.page_info { crate::marks::page_info(doc, &title, i, now) } else { String::new() };
        let art = crate::marks::art(doc, &marks, &boxes, bleed, &info);
        // The page is the media box, y down from its top-left corner (like ours), in units of
        // `unit` points.
        let unit = if scales { user_unit(m.width(), m.height()) } else { 1.0 };
        units.push(unit);
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
            area: r,
            // Art reaches as far as the bleed: the marks lie outside it.
            clip: m != r,
            marks: art.as_ref(),
            negative: false,
        };
        w.page(&mut ex, &sheet).map_err(|_| PdfError::BadArtboard(i))?;
    }
    vectorcraft_doc::cancel::check().map_err(|_| PdfError::Cancelled)?;
    let (layers, overprinted, invisible) = (ex.layers, ex.overprinted, ex.invisible);
    w.absorb(ex);
    let (bytes, more) = w.finish(left_out, display_doc_title)?;
    let bytes = crate::forms::finish(bytes, doc, set, layers, overprinted, invisible)?;
    // The writer gives the file its creation date as its modification date too.
    let bytes = match opts.modified.filter(|m| Some(*m) != created_at) {
        Some(modified) => crate::post::modification_date(bytes, modified)?,
        None => bytes,
    };
    // The writer's metadata has no copyright: it is added to the XMP packet.
    let bytes = crate::post::rights(bytes, &doc.metadata)?;
    warnings.extend(more);
    let bytes = crate::post::user_units(bytes, &units)?;
    let bytes = crate::post::finish(bytes, opts, native.is_some(), &mut warnings)?;
    warnings.dedup();
    Ok(ExportReport { bytes, warnings })
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

/// The `UserUnit` (PDF 1.6) of a page `w` × `h` points large: 1 when both sides are at most
/// [`MAX_PAGE_SIDE`], else the longer side divided by it, rounded up to the next thousandth, so
/// that both sides are at most [`MAX_PAGE_SIDE`] units.
fn user_unit(w: f64, h: f64) -> f64 {
    let side = w.abs().max(h.abs());
    if side.is_finite() && side > MAX_PAGE_SIDE { (side / MAX_PAGE_SIDE * 1000.0).ceil() / 1000.0 } else { 1.0 }
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
