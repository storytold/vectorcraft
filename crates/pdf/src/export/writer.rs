use super::entry::RgbInCmyk;
use super::exporter::{constant_mask, cover, to_path, writable, xf};
use super::{
    Affine, Arc, Archival, ColorOut, Compatibility, Document, Exporter, HashSet, Image, Metadata, Node, PageSettings, PdfError, PdfVersion, Rect,
    Shape, Size, Standard, find, krilla, rfind, write,
};

/// A PDF being written, shared by export and print: the file's configuration and metadata, then
/// its pages ([`Self::page`]), each drawn by an [`Exporter`].
pub(crate) struct Writer {
    pub pdf: write::Writer,
    /// How colours are written (the Output settings).
    pub out: Arc<ColorOut>,
    /// Groups blend in CMYK (a CMYK document's composite, or converting to CMYK; see
    /// `cmyk_blending`).
    cmyk: bool,
    /// Spot colours written with a Lab alternate, gathered from the exporters ([`Self::absorb`]).
    lab_spots: Vec<(String, vectorcraft_color::cms::Lab)>,
    warnings: Vec<String>,
    /// RGB content of a CMYK document, gathered from the exporters ([`Self::absorb`]).
    rgb_in_cmyk: RgbInCmyk,
    /// A PDF 1.3 file ([`crate::PdfSettings::pdf13`]): flat, without page groups.
    pdf13: bool,
    /// The document's images the exporters wrote, by key, gathered from the exporters
    /// ([`Self::absorb`]): [`Self::finish`] finds the key of an image the writer couldn't decode.
    images: Vec<(String, Image)>,
}

/// One page as [`Writer::page`] draws it: the art placed in a drawing space ([`Self::place`]),
/// which [`Self::view`] maps onto the page.
pub(crate) struct Sheet<'a> {
    /// The page's size in its units.
    pub size: (f64, f64),
    /// The size of the page's unit in points (its `UserUnit`): a page in points has 1. A side is
    /// written at least 1 pt long.
    pub unit: f64,
    /// The TrimBox and BleedBox, in page space (y down from the top-left corner).
    pub trim: Rect,
    pub bleed: Rect,
    /// Drawing space → page space.
    pub view: Affine,
    /// The part of the drawing space that shows (a tile): art and marks are clipped to it.
    pub window: Option<Rect>,
    /// Document space → drawing space.
    pub place: Affine,
    /// The art drawn, in document space (the bleed box): what lies outside it is left out.
    pub area: Rect,
    /// Clip the art to [`Self::area`].
    pub clip: bool,
    /// Printer's marks, in drawing space.
    pub marks: Option<&'a Node>,
    /// Invert the page: paper black, ink clear (a film negative).
    pub negative: bool,
}

impl Writer {
    /// A PDF with `set`'s version, standard, compression and colour output, and `doc`'s metadata
    /// under `title`, created at `created` (Unix seconds). With `cmyk`, transparency blends in
    /// CMYK (unless the colours are converted to RGB).
    pub(crate) fn new(doc: &Document, set: &crate::PdfSettings, title: &str, created: Option<i64>, cmyk: bool) -> Result<Self, PdfError> {
        let out = ColorOut::new(doc, set)?;
        let mut warnings = out.profile_notes.clone();
        // Tagged CMYK colours need the profile (PDF/A tags every colour).
        let cmyk_profile = if out.tagged { out.cmyk_icc() } else { None };
        if out.tagged && cmyk_profile.is_none() {
            warnings.push(format!("the CMYK profile {} can't be embedded: CMYK colours are written untagged", out.cmyk_profile()));
        }
        let version = match set.compatibility {
            // PDF 1.3 files are written with the PDF 1.4 settings: flat, they use nothing newer.
            Compatibility::Pdf13 | Compatibility::Pdf14 => PdfVersion::Pdf14,
            Compatibility::Pdf15 => PdfVersion::Pdf15,
            Compatibility::Pdf16 => PdfVersion::Pdf16,
            Compatibility::Pdf17 => PdfVersion::Pdf17,
            Compatibility::Pdf20 => PdfVersion::Pdf20,
        };
        // `check` has refused the standards (and standard/version pairs) the writer can't produce.
        let archival = (set.standard == Standard::PdfA2b).then_some(Archival::A2_B);
        let configuration = write::configuration(version, archival)?;
        let settings = krilla::SerializeSettings {
            compress_content_streams: set.compression.compress_text,
            no_device_cs: out.tagged,
            cmyk_profile,
            configuration,
            // SVG glyphs, which krilla can't draw, are drawn as art where they sit (`glyph_text`):
            // as text they draw nothing, rather than their fonts' plain outlines over the art.
            render_svg_glyph_fn: |_, _, _, _, _| Some(()),
            ..Default::default()
        };

        // File Info normalization stays with the application.
        let info = &doc.metadata;
        let given = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(str::to_string);
        let metadata = Metadata {
            creator: "VectorCraft".into(),
            producer: "VectorCraft".into(),
            title: Some(title.to_owned()),
            authors: given(&info.author).into_iter().collect(),
            description: given(&info.description),
            keywords: info.keywords.clone(),
            created: created.map(date_time),
            modified: None,
        };
        let pdf = write::Writer::new(settings, metadata);
        Ok(Self {
            pdf,
            cmyk: out.blends_cmyk(cmyk),
            out: Arc::new(out),
            lab_spots: vec![],
            warnings,
            rgb_in_cmyk: RgbInCmyk::default(),
            pdf13: set.pdf13(),
            images: vec![],
        })
    }

    /// Draw one page: the art of `ex`'s document placed as `sheet` says, then its marks.
    pub(crate) fn page(&mut self, ex: &mut Exporter, sheet: &Sheet) -> Result<(), PdfError> {
        let (w, h) = sheet.size;
        let least = 1.0 / sheet.unit;
        let size = Size::from_wh(w.max(least) as f32, h.max(least) as f32).ok_or_else(|| PdfError::BadSetting(format!("page size {w} × {h}")))?;
        let at = |b: Rect| krilla::geom::Rect::from_ltrb(b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32);
        let mut page = self.pdf.document_mut().start_page_with(PageSettings::new(size).with_trim_box(at(sheet.trim)).with_bleed_box(at(sheet.bleed)));
        let mut s = page.surface();
        let paper = Rect::new(0.0, 0.0, w, h);
        if sheet.negative {
            cover(&mut s, paper, 255, 1.0);
        }
        // A page whose art or marks would be placed beyond [`MAX_COORD`] is left blank.
        let fits = writable(&s, sheet.view, Rect::ZERO) && writable(&s, sheet.view * sheet.place, Rect::ZERO);
        s.push_transform(&xf(sheet.view));
        let window = sheet.window.and_then(|r| to_path(&s, &r.to_path(0.1)));
        if let Some(clip) = &window {
            s.push_clip_path(clip, &krilla::paint::FillRule::NonZero);
        }
        let placed = sheet.place != Affine::IDENTITY;
        if placed {
            s.push_transform(&xf(sheet.place));
        }
        let r = sheet.area;
        let clip = sheet.clip.then(|| to_path(&s, &r.to_path(0.1))).flatten();
        if let Some(clip) = &clip {
            s.push_clip_path(clip, &krilla::paint::FillRule::NonZero);
        }
        // Page Isolated Blending / Page Knockout Group: the page content is one group (the PDF
        // writer has no page group attributes; a flat PDF 1.3 file has none). In CMYK, transparency
        // at the top of a page is put in a group of its own, whose blending space `cmyk_blending`
        // rewrites: non-isolated only when blending on the page reaches the paper, since some
        // readers (CoreGraphics) drop most of a page in a non-isolated group (#2156).
        let doc = ex.doc;
        let page_group = fits && !self.pdf13 && (doc.page_isolate || doc.page_knockout);
        let cmyk_page_group = fits && self.cmyk && !page_group && ex.shows_transparency();
        if page_group || (cmyk_page_group && !Node::children_blend(&doc.layers, &mut |c| c.blends_through())) {
            s.push_isolated();
        } else if cmyk_page_group {
            let all = constant_mask(&mut s, r, 1.0);
            s.push_mask(all);
        }
        if fits {
            ex.children(&mut s, &doc.layers, r);
        }
        if page_group || cmyk_page_group {
            s.pop();
        }
        if clip.is_some() {
            s.pop();
        }
        if placed {
            s.pop();
        }
        if fits && let Some(art) = sheet.marks {
            // The drawing space the page shows (the marks' area).
            let shown = sheet.view.inverse().transform_rect_bbox(paper);
            let knockout = std::mem::replace(&mut ex.knockout, false);
            // Marks keep their inks: they aren't converted to the destination. The page
            // information is drawn as glyph shapes, like the other marks, not as text.
            let convert = std::mem::replace(&mut ex.convert, false);
            let outline = std::mem::replace(&mut ex.outline_text, true);
            ex.node(&mut s, art, sheet.window.map_or(shown, |r| r.intersect(shown)), true);
            (ex.knockout, ex.convert, ex.outline_text) = (knockout, convert, outline);
        }
        if window.is_some() {
            s.pop();
        }
        s.pop();
        if sheet.negative {
            s.push_blend_mode(krilla::blend::BlendMode::Difference);
            cover(&mut s, paper, 255, 1.0);
            s.pop();
        }
        s.finish();
        page.finish();
        Ok(())
    }

    /// Take an exporter's warnings, Lab spot colours and images once its pages are drawn.
    pub(crate) fn absorb(&mut self, ex: Exporter) {
        for w in ex.warnings {
            if !self.warnings.contains(&w) {
                self.warnings.push(w);
            }
        }
        self.rgb_in_cmyk.merge(ex.rgb_in_cmyk);
        for spot in ex.lab_spots {
            if !self.lab_spots.iter().any(|(n, _)| *n == spot.0) {
                self.lab_spots.push(spot);
            }
        }
        // The exporter caches images by their key, followed by their size and mask after a NUL.
        for (cache, image) in ex.images {
            if let (Some(key), Some(image)) = (cache.split('\0').next(), image) {
                self.images.push((key.to_string(), image));
            }
        }
    }

    /// The finished file (with the output intent and Trapped entries, and checked against its
    /// standard) and the warnings of its drawing. PDF/X-1a spot colours keep their CMYK
    /// alternates. When the writer couldn't decode one of the images, its key is added to
    /// `left_out` ([`leaving_out_undecodable`]) and the writer's error is returned.
    pub(crate) fn finish(mut self, left_out: &mut HashSet<String>, display_doc_title: bool) -> Result<(Vec<u8>, Vec<String>), PdfError> {
        let images = std::mem::take(&mut self.images);
        let mut bytes = self.pdf.finish().map_err(|e| {
            if let write::WriteError::Serialization(k) = &e
                && let krilla::error::KrillaError::Image(failed, ..) = &**k
            {
                left_out.extend(images.iter().filter(|(_, image)| image == failed).map(|(key, _)| key.clone()));
            }
            PdfError::from(e)
        })?;
        if self.cmyk {
            cmyk_blending(&mut bytes, self.out.tagged);
        }
        let lab = !self.lab_spots.is_empty() && !self.out.standard.cmyk_only();
        let bytes = if lab { crate::lab_spot::lab_alternates(bytes, &self.lab_spots) } else { bytes };
        let (bytes, more) = self.out.write_catalog(bytes, display_doc_title)?;
        self.warnings.extend(more);
        self.warnings.extend(self.rgb_in_cmyk.warning());
        Ok((crate::pdfx::finish(bytes, self.out.standard, self.pdf13)?, self.warnings))
    }
}

/// Make the transparency groups of `pdf` blend in CMYK. The PDF writer gives every group RGB as
/// its blending space (DeviceRGB, or sRGB when colours are `tagged`), so the group dictionaries
/// are rewritten in place to DeviceCMYK (or the CMYK profile's space), keeping their length so the
/// cross-reference offsets stay valid. Luminosity masks' groups keep RGB: mask luminance is that
/// of screen colours, as on screen.
fn cmyk_blending(pdf: &mut [u8], tagged: bool) {
    let masks = luminosity_mask_groups(pdf);
    let mut spaces = vec![("/CS/DeviceRGB".to_string(), "/CS/DeviceCMYK".to_string())];
    if tagged {
        spaces.extend(icc_spaces(pdf));
    }
    let mut from = 0;
    while let Some(at) = find(pdf, b"/Group<<", from).map(|i| i + b"/Group".len()) {
        let end = find(pdf, b">>", at).map_or(pdf.len(), |i| i + 2);
        let mask = pdf.get(..at).and_then(object_number).is_some_and(|n| masks.contains(&n));
        if let (false, Some(dict)) = (mask, pdf.get_mut(at..end)) {
            cmyk_group(dict, &spaces);
        }
        from = end;
    }
}

/// The ICC-based RGB colour spaces of `pdf`, each with the ICC-based CMYK one to blend in instead,
/// as group `/CS` entries (`/CS 3 0 R`); none without a CMYK one.
fn icc_spaces(pdf: &[u8]) -> Vec<(String, String)> {
    const ICC: &[u8] = b"[/ICCBased ";
    let (mut rgb, mut cmyk) = (vec![], None);
    let mut from = 0;
    while let Some(at) = find(pdf, ICC, from) {
        from = at + ICC.len();
        let digits = pdf.get(from..).map_or(0, |r| r.iter().take_while(|b| b.is_ascii_digit()).count());
        let (Some(array), Some(stream)) = (pdf.get(..at).and_then(object_number), pdf.get(from..from + digits).and_then(number)) else {
            continue;
        };
        // The profile stream's dictionary says how many components it has.
        let Some(head) = find(pdf, format!("\n{stream} 0 obj").as_bytes(), 0) else { continue };
        let dict = pdf.get(head..find(pdf, b"stream", head).unwrap_or(pdf.len())).unwrap_or_default();
        if find(dict, b"/N 4", 0).is_some() {
            cmyk.get_or_insert(format!("/CS {array} 0 R"));
        } else if find(dict, b"/N 3", 0).is_some() {
            rgb.push(format!("/CS {array} 0 R"));
        }
    }
    let Some(cmyk) = cmyk else { return vec![] };
    rgb.into_iter().map(|r| (r, cmyk.clone())).collect()
}

/// Rewrite transparency group dictionary `dict` (`<<…>>`) to blend in the CMYK space of the first
/// of `spaces` (RGB `/CS` entry, CMYK one) it has, at the same length: leaving out the optional
/// `/Type/Group` makes room for the longer entry, spaces pad the rest.
fn cmyk_group(dict: &mut [u8], spaces: &[(String, String)]) {
    let Ok(text) = std::str::from_utf8(dict) else { return };
    if !(text.contains("/S/Transparency") && text.contains("/Type/Group")) {
        return;
    }
    let Some((rgb, cmyk)) = spaces.iter().find(|(rgb, _)| text.contains(rgb.as_str())) else { return };
    let body = text.replacen("/Type/Group", "", 1).replacen(rgb.as_str(), cmyk, 1);
    let body = body.strip_suffix(">>").unwrap_or(&body);
    let Some(pad) = dict.len().checked_sub(body.len() + 2) else { return };
    let new = format!("{body}{}>>", " ".repeat(pad));
    dict.copy_from_slice(new.as_bytes());
}

/// Object numbers of the groups luminosity soft masks draw (`/G n 0 R` in their dictionaries).
fn luminosity_mask_groups(pdf: &[u8]) -> Vec<u32> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(at) = find(pdf, b"/Type/Mask", from) {
        let end = find(pdf, b">>", at).unwrap_or(pdf.len());
        let dict = &pdf[at..end];
        if find(dict, b"/S/Luminosity", 0).is_some()
            && let Some(g) = find(dict, b"/G ", 0)
        {
            let rest = &dict[g + 3..];
            out.extend(number(&rest[..rest.iter().take_while(|b| b.is_ascii_digit()).count()]));
        }
        from = end;
    }
    out
}

/// The number of the object whose dictionary `before` ends in (its last `n 0 obj` header).
fn object_number(before: &[u8]) -> Option<u32> {
    let at = rfind(before, b" 0 obj")?;
    let digits = before[..at].iter().rev().take_while(|b| b.is_ascii_digit()).count();
    number(&before[at - digits..at])
}

fn number(digits: &[u8]) -> Option<u32> {
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// Unix seconds (UTC) → krilla date.
pub(super) fn date_time(t: i64) -> krilla::metadata::DateTime {
    let [year, month, day, hour, minute, second] = vectorcraft_doc::metadata::civil(t);
    krilla::metadata::DateTime::new(year as u16)
        .month(month as u8)
        .day(day as u8)
        .hour(hour as u8)
        .minute(minute as u8)
        .second(second as u8)
        .utc_offset_hour(0)
        .utc_offset_minute(0)
}
