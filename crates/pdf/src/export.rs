//! Document → PDF (krilla). Mirrors the tree walk of `vectorcraft-render`.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft_pdf::write::backend as krilla;
use craft_pdf::write::{self, Metadata};
use krilla::color::separation::{Color as SepColor, SeparationColorant, SeparationSpace};
use krilla::color::{cmyk, luma, rgb};
use krilla::configure::{Archival, PdfVersion};
use krilla::geom::{Path, PathBuilder, Size, Transform};
use krilla::image::Image;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, LinearGradient, RadialGradient, SpreadMethod, Stop, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{Glyph, GlyphId, KrillaGlyph};
use kurbo::{PathEl, Shape, Vec2};
use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, Document, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, SymbolBoxes, TextObject};
use vectorcraft_effects::stroke::{self, WrittenShape};
use vectorcraft_geom::{Affine, BezPath, FillRule, Rect};
use vectorcraft_text::TextLayout;
use vectorcraft_text::embed::Embedding;

use crate::forms::Mark;
use crate::lab_spot::{find, rfind};
use crate::marks::PageBoxes;
use crate::output::ColorOut;
use crate::{Compatibility, ExportReport, PdfError, PdfOptions, Standard};

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
    colours: HashSet<[u32; 3]>,
    /// Keys of the raster images that aren't CMYK.
    images: HashSet<String>,
}

impl RgbInCmyk {
    fn merge(&mut self, other: RgbInCmyk) {
        self.colours.extend(other.colours);
        self.images.extend(other.images);
    }

    /// The warning for the export report; `None` when the document has nothing of the sort.
    fn warning(&self) -> Option<String> {
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
fn date_time(t: i64) -> krilla::metadata::DateTime {
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

pub(crate) struct Exporter<'a> {
    doc: &'a Document,
    /// The art boxes of `doc`'s symbols, which the walk measures symbol instances by when it
    /// leaves out objects outside the page ([`SymbolBoxes::quick`]). The box of an instance whose
    /// art holds brushed strokes grows by as far as their brush art can paint
    /// ([`SymbolBoxes::with_brush_art`]).
    symbols: SymbolBoxes<'a>,
    warnings: Vec<String>,
    images: HashMap<String, Option<Image>>,
    /// The brush library, parsed when the first brushed stroke is written.
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// Whether the group being written is a knockout group (what its neutral children inherit).
    knockout: bool,
    /// Spot colours written with a Lab alternate ([`crate::lab_spot`]): colorant name, Lab values.
    lab_spots: Vec<(String, vectorcraft_color::cms::Lab)>,
    /// Images may ask viewers to smooth them (not in PDF/A).
    interpolate: bool,
    /// Images may have 16 bits a sample (PDF 1.5 and later); in earlier files they are written at
    /// 8 bits ([`embed`]).
    sixteen_bit: bool,
    /// How images are resampled and compressed.
    compression: &'a crate::CompressionSettings,
    /// Layers whose Print option is off are written too.
    pub non_printing: bool,
    /// How colours are separated into CMYK (the colour settings' intent unless print sets one).
    pub intent: vectorcraft_color::cms::Intent,
    /// Whether a layer shows transparency (found out on the first page that asks).
    transparent: Option<bool>,
    /// How colours are written (the Output settings; print writes them as they are).
    pub out: Arc<ColorOut>,
    /// Colours are converted to the destination (printer's marks aren't).
    convert: bool,
    /// RGB colours and images of a CMYK document drawn so far.
    rgb_in_cmyk: RgbInCmyk,
    /// Text as glyph outlines; else the characters' fills are real text in embedded fonts.
    outline_text: bool,
    /// The fonts real text is written in, by face id, with their units per em; `None` for faces
    /// that can't be embedded.
    fonts: HashMap<u32, Option<(krilla::text::Font, f64)>>,
    /// Layers and sublayers are drawn as forms marked for their optional content groups
    /// ([`crate::forms`]).
    pub layers: bool,
    /// Each layer's index in [`crate::forms::pdf_layers`], by address.
    layer_index: HashMap<usize, usize>,
    /// The address of the layer whose form is being drawn.
    in_layer: usize,
    /// Overprinting fills and strokes are drawn as forms marked to overprint.
    pub overprint: bool,
    /// An overprinting fill or stroke was drawn so.
    pub overprinted: bool,
    /// Characters with neither fill nor stroke, or whose fill is drawn as art (outlines filled
    /// with an image or pattern), were written as text in forms marked invisible.
    pub invisible: bool,
    /// Inline graphics being drawn inside inline graphics (text in a symbol shown inline in
    /// text…): deeper ones are left out.
    inline_depth: u32,
    /// Tiling patterns being drawn inside tiling patterns ([`Self::tiling`]).
    pattern_depth: u32,
    /// Keys of the images left out because the writer couldn't decode them
    /// ([`leaving_out_undecodable`]).
    pub left_out: HashSet<String>,
    /// Objects the symbol instances written held, together
    /// ([`vectorcraft_doc::MAX_SYMBOL_NODES`]).
    symbol_nodes: u64,
}

/// How deep inline graphics nest before they are left out.
const MAX_INLINE_DEPTH: u32 = 4;

/// How deep tiling patterns nest: deeper pattern fills are written as their tiles.
const MAX_PATTERN_DEPTH: u32 = 4;

/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";

impl<'a> Exporter<'a> {
    /// An exporter of `doc` writing images as `set` says; layers whose Print option is off are
    /// left out.
    pub(crate) fn new(doc: &'a Document, set: &'a crate::PdfSettings) -> Self {
        Self {
            doc,
            symbols: doc.symbol_boxes().quick().with_brush_art(),
            warnings: vec![],
            images: HashMap::new(),
            brushes: None,
            knockout: doc.page_knockout,
            lab_spots: vec![],
            interpolate: set.standard != Standard::PdfA2b,
            // PDF 1.3 and 1.4 files are written with the PDF 1.4 settings ([`Writer::new`]).
            sixteen_bit: !matches!(set.compatibility, Compatibility::Pdf13 | Compatibility::Pdf14),
            compression: &set.compression,
            non_printing: false,
            intent: vectorcraft_color::cms::active().settings().intent,
            transparent: None,
            out: Arc::default(),
            convert: true,
            rgb_in_cmyk: RgbInCmyk::default(),
            outline_text: set.advanced.outline_text,
            fonts: HashMap::new(),
            layers: false,
            layer_index: crate::forms::pdf_layers(doc).iter().enumerate().map(|(i, l)| (address(l.node), i)).collect(),
            in_layer: 0,
            symbol_nodes: 0,
            overprint: false,
            overprinted: false,
            invisible: false,
            inline_depth: 0,
            pattern_depth: 0,
            left_out: HashSet::new(),
        }
    }

    /// Whether a layer shows transparency, which CMYK pages put in a group of its own.
    fn shows_transparency(&mut self) -> bool {
        let doc = self.doc;
        *self.transparent.get_or_insert_with(|| doc.layers.iter().any(|l| l.shows_transparency()))
    }
}

/// Glyphs written together as real text: the same font at the same size along one baseline.
struct GlyphLine {
    face: u32,
    font: krilla::text::Font,
    size: f64,
    /// The glyph space (krilla's, y down, at `size`) → text space, at the line's start.
    frame: Affine,
    /// Glyph id, pen position and advance along the line (in glyph space), and the characters it
    /// stands for (bytes of the plain text).
    glyphs: Vec<(u32, f64, f64, std::ops::Range<usize>)>,
}

/// Most pixels along a side of a freeform gradient's image (its colour field is smooth).
const MAX_FIELD_PX: f64 = 512.0;
/// Most pixels along a side of a linear or radial gradient's image.
const MAX_GRADIENT_PX: f64 = 4096.0;

/// The largest magnitude a coordinate, length or transform entry may have in the content stream it
/// is drawn into (for glyph spacing, in thousandths of an em). Art that would go beyond it is left
/// out. A PDF/A-2 file may hold only integers from -2,147,483,648 to 2,147,483,647 (ISO 19005-2,
/// 6.1.13). The PDF writer writes a whole number in the `i32` range as an integer, and in builds
/// with overflow checks it panics on -2,147,483,648.
const MAX_COORD: f64 = 1e9;

fn xf(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

/// The current transform of surface `s` in the content stream it draws into, without that stream's
/// root transform (a page's y flip).
fn ctm(s: &Surface) -> Affine {
    let t = s.ctm();
    Affine::new([t.sx(), t.ky(), t.kx(), t.sy(), t.tx(), t.ty()].map(f64::from))
}

/// Are `values` finite and within [`MAX_COORD`]?
fn in_range(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

/// Can art spanning `b`, in the space `space` maps into the one `s` draws in, be written: `b`, the
/// transform of `s` followed by `space`, and `b` through that transform all within [`MAX_COORD`]?
fn writable(s: &Surface, space: Affine, b: Rect) -> bool {
    let t = ctm(s) * space;
    let to = t.transform_rect_bbox(b);
    in_range(&t.as_coeffs()) && in_range(&[b.x0, b.y0, b.x1, b.y1, to.x0, to.y0, to.x1, to.y1])
}

/// `bp` as a path to draw on `s`; `None` when it is empty or reaches beyond [`MAX_COORD`] there.
fn to_path(s: &Surface, bp: &BezPath) -> Option<Path> {
    if !writable(s, Affine::IDENTITY, bp.control_box()) {
        return None;
    }
    let mut pb = PathBuilder::new();
    for el in bp.elements() {
        match *el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => pb.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, b, p) => pb.cubic_to(a.x as f32, a.y as f32, b.x as f32, b.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

fn norm(v: f32) -> NormalizedF32 {
    NormalizedF32::new(if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }).unwrap_or(NormalizedF32::ONE)
}

fn rule(r: FillRule) -> krilla::paint::FillRule {
    match r {
        FillRule::NonZero => krilla::paint::FillRule::NonZero,
        FillRule::EvenOdd => krilla::paint::FillRule::EvenOdd,
    }
}

fn blend(b: BlendMode) -> krilla::blend::BlendMode {
    use krilla::blend::BlendMode as K;
    match b {
        BlendMode::Normal => K::Normal,
        BlendMode::Darken => K::Darken,
        BlendMode::Multiply => K::Multiply,
        BlendMode::ColorBurn => K::ColorBurn,
        BlendMode::Lighten => K::Lighten,
        BlendMode::Screen => K::Screen,
        BlendMode::ColorDodge => K::ColorDodge,
        BlendMode::Overlay => K::Overlay,
        BlendMode::SoftLight => K::SoftLight,
        BlendMode::HardLight => K::HardLight,
        BlendMode::Difference => K::Difference,
        BlendMode::Exclusion => K::Exclusion,
        BlendMode::Hue => K::Hue,
        BlendMode::Saturation => K::Saturation,
        BlendMode::Color => K::Color,
        BlendMode::Luminosity => K::Luminosity,
    }
}

fn q(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn color(c: &Color) -> krilla::color::Color {
    match *c {
        Color::Rgb { r, g, b } => rgb::Color::new(q(r), q(g), q(b)).into(),
        Color::Cmyk { c, m, y, k } => cmyk::Color::new(q(c), q(m), q(y), q(k)).into(),
        // VectorCraft grey is ink coverage (0 = white); PDF DeviceGray is lightness.
        Color::Gray { k } => luma::Color::new(q(1.0 - k)).into(),
        Color::Lab { .. } => {
            let [r, g, b] = c.to_rgb();
            rgb::Color::new(q(r), q(g), q(b)).into()
        }
    }
}

/// Fill the page (with a margin) with a grey level (0 = black, 255 = white) at `opacity`: mask
/// backdrops.
fn cover(s: &mut Surface, page: Rect, level: u8, opacity: f32) {
    let Some(p) = to_path(s, &page.inflate(1.0, 1.0).to_path(0.1)) else { return };
    s.set_stroke(None);
    s.set_fill(Some(Fill { paint: rgb::Color::new(level, level, level).into(), opacity: norm(opacity), rule: krilla::paint::FillRule::NonZero }));
    s.draw_path(&p);
}

/// An alpha mask of constant `alpha` over the page. A group drawn through it is a transparency
/// group of that opacity that, unlike the writer's opacity groups, is not isolated.
/// The cover is white, so a reader that takes every soft mask as a luminosity mask shows the same.
fn constant_mask(s: &mut Surface, page: Rect, alpha: f32) -> krilla::mask::Mask {
    let mut sb = s.stream_builder();
    let mut ms = sb.surface();
    cover(&mut ms, page, 255, alpha);
    ms.finish();
    krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Alpha)
}

/// A node's address, to tell the very node apart from equal ones.
fn address(n: &Node) -> usize {
    std::ptr::from_ref(n).addr()
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

impl Exporter<'_> {
    /// A colour for the page, as the Output settings write it ([`ColorOut::color`]): without
    /// conversion, RGB and Lab colours of CMYK documents are separated into CMYK through the
    /// document's colour spaces, so the file carries press values.
    fn col(&mut self, c: &Color) -> krilla::color::Color {
        let cmyk_doc = self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
        if let (true, true, Color::Rgb { r, g, b }) = (cmyk_doc, self.convert, c) {
            self.rgb_in_cmyk.colours.insert([r.to_bits(), g.to_bits(), b.to_bits()]);
        }
        let c = self.out.color(c, self.intent, cmyk_doc, self.convert);
        if let (Color::Rgb { .. }, Some(note)) = (c, &self.out.srgb_note) {
            let note = note.clone();
            self.warn(note);
        }
        color(&c)
    }

    /// The Separation colour space of spot swatch `name` (its CMYK equivalent is the alternate
    /// space; a Lab spot colour also gets a Lab one, [`crate::lab_spot`], unless the Spot Colors
    /// options use CMYK values); `None` when `name` isn't a spot colour.
    fn separation(&mut self, name: &str) -> Option<SeparationSpace> {
        let doc = self.doc;
        let sw = doc.swatch(name).filter(|s| s.spot)?;
        let color = doc.linked_color(sw.paint.color()?, true);
        if let Color::Lab { l, a, b } = color
            && !self.lab_spots.iter().any(|(n, _)| n == name)
        {
            self.lab_spots.push((name.to_string(), vectorcraft_color::cms::Lab::new(l, a, b)));
        }
        let full = self.out.cmyk(&color, self.intent);
        let alt = krilla::color::RegularColor::Cmyk(cmyk::Color::new(q(full[0]), q(full[1]), q(full[2]), q(full[3])));
        Some(SeparationSpace::new(SeparationColorant::Custom(sw.name.clone()), alt))
    }

    /// A solid paint, in the Separation colour space at its tint when it's linked to a spot swatch
    /// or to Registration (`/All`: every plate).
    fn solid(&mut self, c: &Color, link: Option<&str>, tint: f32) -> krilla::color::Color {
        if link == Some(vectorcraft_color::swatch::REGISTRATION) {
            let alt = krilla::color::RegularColor::Cmyk(cmyk::Color::new(255, 255, 255, 255));
            return SepColor::new(q(tint), SeparationSpace::new(SeparationColorant::AllColorants, alt)).into();
        }
        match link.and_then(|n| self.separation(n)) {
            Some(space) => SepColor::new(q(tint), space).into(),
            None => self.col(c),
        }
    }

    /// The stops of a gradient whose every stop is a tint of one spot ink (or paper white, 0% of
    /// it) as tints of that ink — a Separation shading — with midpoints as explicit stops (the
    /// tint and opacity there are halfway). `None` for other gradients; one that mixes a spot ink
    /// with other colours is written in process colours (the PDF writer has no DeviceN).
    fn spot_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<Vec<(f32, krilla::color::Color, f32)>> {
        let ink = g.stops.iter().find_map(|s| s.swatch.as_deref().filter(|n| self.doc.swatch(n).is_some_and(|w| w.spot)))?;
        let paper = |c: &Color| c.to_rgba8(1.0)[..3] == [255; 3];
        let tints: Option<Vec<f32>> = g
            .stops
            .iter()
            .map(|s| match s.swatch.as_deref() {
                Some(n) if n == ink => Some(s.tint),
                None if paper(&s.color) => Some(0.0),
                _ => None,
            })
            .collect();
        let Some(tints) = tints else {
            self.warn("gradients mixing a spot color with other colors are exported in process colors");
            return None;
        };
        let space = self.separation(ink)?;
        let tint = |t: f32| -> krilla::color::Color { SepColor::new(q(t), space.clone()).into() };
        let mut out = vec![];
        for (i, s) in g.stops.iter().enumerate() {
            out.push((s.offset, tint(tints[i]), s.opacity));
            if let Some(n) = g.stops.get(i + 1)
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                let at = s.offset + (n.offset - s.offset) * s.midpoint;
                out.push((at, tint((tints[i] + tints[i + 1]) / 2.0), (s.opacity + n.opacity) / 2.0));
            }
        }
        Some(out)
    }

    /// Warn when `effects` (an object's, a fill's or a stroke's) has a visible raster effect: the
    /// app renders them to images before export (`vectorcraft_engine::export_pdf`), so the ones
    /// still here (on `what`) can't be written.
    fn warn_raster(&mut self, effects: &[vectorcraft_doc::Effect], what: impl FnOnce() -> String) {
        if effects.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id)) {
            self.warn(format!("raster effects (shadows, glows, blur, feather) on {} are left out of the PDF", what()));
        }
    }

    fn warn(&mut self, w: impl Into<String>) {
        let w = w.into();
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    /// Convert a paint to draw on `s`. `bounds` resolves unset gradient geometry (like the
    /// renderer). A gradient placed beyond [`MAX_COORD`] there gives no paint.
    fn paint(&mut self, s: &Surface, p: &Paint, bounds: Rect) -> Option<krilla::paint::Paint> {
        self.paint_in(s, p, bounds, Affine::IDENTITY)
    }

    /// [`Self::paint`] drawn in another space: `space` maps the paint's space (where `bounds`
    /// is) to the one drawn in.
    fn paint_in(&mut self, s: &Surface, p: &Paint, bounds: Rect, space: Affine) -> Option<krilla::paint::Paint> {
        match p {
            Paint::None => None,
            Paint::Solid { color: c, swatch, tint } => Some(self.solid(c, swatch.as_deref(), *tint).into()),
            Paint::Gradient(g) => {
                let geom = g.resolve(bounds);
                let spot = (g.gradient.kind != GradientKind::Freeform).then(|| self.spot_stops(&g.gradient)).flatten();
                let colored = match spot {
                    Some(v) => v,
                    None => {
                        let stops = g.gradient.expanded_stops();
                        // The PDF writer needs every stop in one colour space: stops of mixed
                        // models (midpoints sample in RGB) go through RGB, or stay CMYK in a CMYK
                        // document, where `col` separates the rest.
                        let mixed = stops.windows(2).any(|w| w[0].1.model() != w[1].1.model());
                        let cmyk_doc = self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
                        let one = |c: Color| match c {
                            Color::Cmyk { .. } if cmyk_doc => c,
                            _ if mixed => c.in_model(vectorcraft_color::cms::Model::Rgb),
                            _ => c,
                        };
                        stops.into_iter().map(|(o, c, a)| (o, self.col(&one(c)), a)).collect()
                    }
                };
                let mut stops: Vec<Stop> = Vec::new();
                let mut last = 0.0f32;
                for (o, color, a) in colored {
                    let o = o.clamp(last, 1.0);
                    last = o;
                    stops.push(Stop { offset: norm(o), color, opacity: norm(a) });
                }
                if stops.is_empty() {
                    return None;
                }
                match g.gradient.kind {
                    GradientKind::Linear => {
                        if !writable(s, space, Rect::from_points(geom.start, geom.end)) {
                            return None;
                        }
                        let (s, mut e) = (geom.start, geom.end);
                        if s.distance(e) < 1e-9 {
                            e = s + Vec2::new(1.0, 0.0);
                        }
                        Some(
                            LinearGradient {
                                x1: s.x as f32,
                                y1: s.y as f32,
                                x2: e.x as f32,
                                y2: e.y as f32,
                                transform: xf(space),
                                spread_method: SpreadMethod::Pad,
                                stops,
                                anti_alias: false,
                            }
                            .into(),
                        )
                    }
                    GradientKind::Radial => {
                        let r = geom.length().max(1e-6) as f32;
                        let t = geom.radial_squash();
                        let (cx, cy) = (geom.start.x as f32, geom.start.y as f32);
                        // An off-centre focal point: a two-point radial shading from it.
                        let f = t.inverse() * geom.focal_point();
                        let circles = Rect::from_center_size(geom.start, (2.0 * f64::from(r), 2.0 * f64::from(r))).union_pt(f);
                        if !writable(s, space * t, circles) {
                            return None;
                        }
                        Some(
                            RadialGradient {
                                fx: f.x as f32,
                                fy: f.y as f32,
                                fr: 0.0,
                                cx,
                                cy,
                                cr: r,
                                transform: xf(space * t),
                                spread_method: SpreadMethod::Pad,
                                stops,
                                anti_alias: false,
                            }
                            .into(),
                        )
                    }
                    // Painted areas get an image of the field ([`Self::area`]); elsewhere (gradient
                    // slices) the average colour stands in.
                    GradientKind::Freeform => {
                        self.warn("freeform gradients are exported as their average colour");
                        let n = g.gradient.stops.len().max(1) as f32;
                        let mut acc = [0.0f32; 3];
                        for s in &g.gradient.stops {
                            let c = s.color.to_rgb();
                            for i in 0..3 {
                                acc[i] += c[i] / n;
                            }
                        }
                        Some(self.col(&Color::rgb(acc[0], acc[1], acc[2])).into())
                    }
                }
            }
            // Patterns paint through [`Self::area`]: only a missing one gets here.
            Paint::Pattern { .. } => {
                self.warn("missing patterns are exported as mid-grey");
                Some(rgb::Color::new(128, 128, 128).into())
            }
        }
    }

    /// Opacity mask → luminosity soft mask. Outside the art the backdrop is black (clip) or white;
    /// invert is a white Difference rect on top (luminance is linear, so luma(1 − c) = 1 − luma(c)).
    fn soft_mask(&mut self, s: &mut Surface, m: &vectorcraft_doc::OpacityMask, page: Rect) -> krilla::mask::Mask {
        let mut sb = s.stream_builder();
        let mut ms = sb.surface();
        // An opaque backdrop when it matters: white outside the art (no clip), or black for the
        // inverting Difference pass below to turn white.
        if !m.clip || m.invert {
            cover(&mut ms, page, if m.clip { 0 } else { 255 }, 1.0);
        }
        // Mask art is a picture of its own: it takes no part in a knockout group around the object,
        // and has no inks to overprint.
        let knockout = std::mem::take(&mut self.knockout);
        let overprint = std::mem::take(&mut self.overprint);
        self.node(&mut ms, &m.art, page, true);
        (self.knockout, self.overprint) = (knockout, overprint);
        if m.invert {
            ms.push_blend_mode(krilla::blend::BlendMode::Difference);
            cover(&mut ms, page, 255, 1.0);
            ms.pop();
        }
        ms.finish();
        krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity)
    }

    /// The children of a group, as the elements of a knockout group when it is one.
    fn children(&mut self, s: &mut Surface, children: &[std::sync::Arc<Node>], page: Rect) {
        if !self.knockout {
            for c in children {
                self.node(s, c, page, false);
            }
            return;
        }
        // The PDF writer has no knockout groups: each element is drawn through a soft mask of
        // where the elements above it don't paint (the same look for Normal blending). The masks
        // nest, so element i sits inside the masks of elements i+1…n: open them outermost first.
        self.warn("knockout groups are written as soft-masked groups (same look, but not editable as knockout groups)");
        let elements: Vec<_> = Node::knockout_elements(children)
            .into_iter()
            .filter(|c| c.visual_bounds_with(&self.symbols).is_some_and(|b| rects_overlap(b.inflate(1.0, 1.0), page)))
            .collect();
        let Some((first, rest)) = elements.split_first() else { return };
        for c in rest.iter().rev() {
            let mask = self.knockout_mask(s, c, page);
            s.push_mask(mask);
        }
        self.node(s, first, page, false);
        for c in rest {
            s.pop();
            self.node(s, c, page, false);
        }
    }

    /// A luminosity mask that is 1 − the knockout shape of `c`: white, then black through an alpha
    /// mask of `c` (at full object opacity without its own mask, unless those define its shape).
    fn knockout_mask(&mut self, s: &mut Surface, c: &Node, page: Rect) -> krilla::mask::Mask {
        let shape = if c.knockout_shape {
            c.clone()
        } else {
            let mut s = c.clone().with_mask(None);
            s.opacity = 1.0;
            s
        };
        let mut sb = s.stream_builder();
        let mut ms = sb.surface();
        cover(&mut ms, page, 255, 1.0);
        let alpha = {
            let mut ab = ms.stream_builder();
            let mut als = ab.surface();
            let overprint = std::mem::take(&mut self.overprint);
            self.node(&mut als, &shape, page, false);
            self.overprint = overprint;
            als.finish();
            krilla::mask::Mask::new(ab.finish(), krilla::mask::MaskType::Alpha)
        };
        ms.push_mask(alpha);
        cover(&mut ms, page, 0, 1.0);
        ms.pop();
        ms.finish();
        krilla::mask::Mask::new(sb.finish(), krilla::mask::MaskType::Luminosity)
    }

    fn node(&mut self, s: &mut Surface, n: &Node, page: Rect, force: bool) {
        if self.layers
            && self.in_layer != address(n)
            && let NodeKind::Layer { printable, .. } = n.kind
            && (printable || self.non_printing)
            && let Some(&i) = self.layer_index.get(&address(n))
        {
            // A PDF layer: hidden layers and sublayers are written too (their group is off).
            let enclosing = std::mem::replace(&mut self.in_layer, address(n));
            crate::forms::form(s, Mark::Layer(i), |s| self.node(s, n, page, true));
            self.in_layer = enclosing;
            return;
        }
        if !force && !n.visible {
            return;
        }
        if let NodeKind::Layer { template, printable, .. } = n.kind
            && (template || !(printable || self.non_printing))
        {
            return;
        }
        match n.visual_bounds_with(&self.symbols) {
            Some(b) if !force && !rects_overlap(b.inflate(1.0, 1.0), page) => return,
            None if !n.is_container() => return,
            _ => {}
        }
        self.warn_raster(&n.appearance.effects, || format!("{} objects", n.kind_label().to_lowercase()));
        let container = n.is_container() && !matches!(n.kind, NodeKind::Compound { .. });
        // Whether this container's children knock each other out (a knockout group is written as a group).
        let knockout = n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        let mut pushes = 0;
        if n.blend != BlendMode::Normal {
            s.push_blend_mode(blend(n.blend));
            pushes += 1;
        }
        if n.opacity < 1.0 || (container && (n.isolate || n.blend != BlendMode::Normal || knockout)) {
            if !n.isolate && n.blends_through() {
                // Not isolated: blending inside reaches the art below the group.
                let alpha = constant_mask(s, page, n.opacity);
                s.push_mask(alpha);
            } else if n.opacity < 1.0 {
                s.push_opacity(norm(n.opacity));
            } else {
                s.push_isolated();
            }
            pushes += 1;
        }
        let mask = n.extras().mask.as_deref().filter(|m| !m.disabled);
        let alpha = match (&n.kind, mask) {
            (NodeKind::Image(im), Some(m)) => self.masked_image(im, m),
            _ => None,
        };
        if let Some(m) = mask
            && alpha.is_none()
        {
            let mask = self.soft_mask(s, m, page);
            s.push_mask(mask);
            pushes += 1;
        }
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => self.children(s, children, page),
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // The region every output clips to; nothing to clip by hides the clipped art. The
                // clipping path's fill paints behind the clipped art and its stroke over it, unclipped.
                if let Some((clip, rest)) = children.split_first()
                    && let Some((p, r)) = vectorcraft_effects::clip_outline(clip).and_then(|(bp, r)| to_path(s, &bp).map(|p| (p, r)))
                {
                    let paint = clip.clip_paint();
                    s.push_clip_path(&p, &rule(r));
                    if let Some(fill) = &paint.fill {
                        self.node(s, fill, page, false);
                    }
                    self.children(s, rest, page);
                    s.pop();
                    if let Some(stroke) = &paint.stroke {
                        self.node(s, stroke, page, false);
                    }
                }
            }
            NodeKind::Path { path, rule, guide, .. } => {
                if !*guide {
                    self.shape(s, n, &path.to_bezpath(), *rule, page);
                }
            }
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for c in children {
                    if let Some(p) = c.path_data() {
                        bp.extend(p.to_bezpath());
                    }
                }
                self.shape(s, n, &bp, *rule, page);
            }
            NodeKind::Text(t) => self.text(s, n, t, page),
            NodeKind::Image(im) => self.image(s, im, alpha),
            NodeKind::SymbolInstance { symbol, xf } => {
                // Within the objects a frame may draw of symbol instances, as on screen: nested
                // instances multiply.
                let sym = self.doc.symbols.iter().find(|x| &x.name == symbol);
                let total = sym.map_or(0, |sym| self.symbol_nodes.saturating_add(sym.art.count() as u64));
                if let Some(sym) = sym.filter(|_| total <= vectorcraft_doc::MAX_SYMBOL_NODES) {
                    self.symbol_nodes = total;
                    let mut art = (*sym.art).clone();
                    art.transform(*xf, false);
                    self.node(s, &art, page, true);
                }
            }
            // Live blends/envelopes/meshes export their evaluated (expanded) form.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(s, c, page, false);
                }
            }
        }
        self.knockout = enclosing;
        for _ in 0..pushes {
            s.pop();
        }
    }

    fn shape(&mut self, s: &mut Surface, n: &Node, bp: &BezPath, r: FillRule, page: Rect) {
        let Some(path) = to_path(s, bp) else { return };
        let bounds = bp.bounding_box();
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    self.warn_raster(&fl.effects, || "fills".into());
                    self.overprinting(s, fl.overprint, &fl.paint, |ex, s| {
                        if ex.area(s, &path, rule(r), &fl.paint, (fl.opacity, fl.blend), bounds, bounds, true) {
                            return;
                        }
                        let Some(paint) = ex.paint(s, &fl.paint, bounds) else { return };
                        let bl = fl.blend != BlendMode::Normal;
                        if bl {
                            s.push_blend_mode(blend(fl.blend));
                        }
                        s.set_stroke(None);
                        s.set_fill(Some(Fill { paint, opacity: norm(fl.opacity), rule: rule(r) }));
                        s.draw_path(&path);
                        if bl {
                            s.pop();
                        }
                    });
                }
                AppearanceItem::Stroke(st) => {
                    if !st.visible || st.paint.is_none() || st.width <= 0.0 {
                        continue;
                    }
                    self.warn_raster(&st.effects, || "strokes".into());
                    self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, bp, &path, r, st, page, bounds));
                }
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
    }

    /// Draw a fill or stroke of `paint` with `draw`: as a form marked to overprint when it
    /// `overprints`, paints and overprints are kept (but not a white one with Discard White
    /// Overprint).
    fn overprinting(&mut self, s: &mut Surface, overprints: bool, paint: &Paint, draw: impl FnOnce(&mut Self, &mut Surface)) {
        let white = || self.doc.setup.discard_white_overprint && vectorcraft_doc::overprint::is_white(paint);
        if overprints && self.overprint && !paint.is_none() && !white() {
            self.overprinted = true;
            crate::forms::form(s, Mark::Overprint, |s| draw(self, s));
        } else {
            draw(self, s);
        }
    }

    /// Is `p` a paint PDF has no paint for when it paints a fill (`fill`) or a stroke, which
    /// [`Self::area`] draws as art: a pattern swatch or a gradient drawn as pixels
    /// ([`vectorcraft_color::GradientPaint::draws_as_pixels`])?
    fn area_paint(&self, p: &Paint, fill: bool) -> bool {
        match p {
            Paint::Pattern { pattern, .. } => self.doc.pattern(pattern).is_some(),
            Paint::Gradient(g) => g.draws_as_pixels(fill),
            _ => false,
        }
    }

    /// Paint the area `clip` (filled by `rule`; bounds `area`) with `p` at `transparency` (opacity,
    /// blend mode) when it is an [area paint](Self::area_paint) of a fill (`fill`) or a stroke: a
    /// pattern's [tiling pattern](Self::tiling) (else its tile instances covering it), or an image
    /// of a gradient drawn as pixels placed on `bounds` (the painted object's box, as on the
    /// canvas), clipped to it. False (nothing drawn) for other paints.
    #[allow(clippy::too_many_arguments)]
    fn area(
        &mut self,
        s: &mut Surface,
        clip: &Path,
        rule: krilla::paint::FillRule,
        p: &Paint,
        transparency: (f32, BlendMode),
        bounds: Rect,
        area: Rect,
        fill: bool,
    ) -> bool {
        if !self.area_paint(p, fill) {
            return false;
        }
        let (opacity, mode) = transparency;
        let tiling = match p {
            Paint::Pattern { pattern, xf } => self.tiling(s, pattern, *xf),
            _ => None,
        };
        let mut pushes = 0;
        if mode != BlendMode::Normal {
            s.push_blend_mode(blend(mode));
            pushes += 1;
        }
        if let Some(paint) = tiling {
            s.set_stroke(None);
            s.set_fill(Some(Fill { paint, opacity: norm(opacity), rule }));
            s.draw_path(clip);
            s.set_fill(None);
        } else {
            s.push_clip_path(clip, &rule);
            pushes += 1;
            if opacity < 1.0 {
                s.push_opacity(norm(opacity));
                pushes += 1;
            }
            let doc = self.doc;
            match p {
                Paint::Pattern { pattern, xf } => {
                    if let Some(def) = doc.pattern(pattern) {
                        self.warn("pattern fills nested more than 4 patterns deep, or with tiles too large for PDF, are written as their tiles");
                        for inst in def.instances_in(*xf, area) {
                            self.node(s, &inst, area, true);
                        }
                    }
                }
                Paint::Gradient(g) => self.field_image(s, g, bounds, area, fill),
                _ => {}
            }
        }
        for _ in 0..pushes {
            s.pop();
        }
        true
    }

    /// Pattern swatch `name` placed by `at` (pattern space → the current space) as a PDF tiling
    /// pattern (ISO 32000-1, 8.7.3.1): one period of the tiling ([`vectorcraft_doc::PatternDef::period`],
    /// what the canvas repeats too) with every tile instance reaching into it, in paint order, so
    /// brick and hex tilings and overlapping tiles repeat as they are; clipped to the period when
    /// art reaches out of it. `None` for a pattern nested deeper than [`MAX_PATTERN_DEPTH`], a period
    /// or placement beyond [`MAX_COORD`] or a placement that flattens the tiles.
    fn tiling(&mut self, s: &mut Surface, name: &str, at: Affine) -> Option<krilla::paint::Paint> {
        let doc = self.doc;
        let def = doc.pattern(name)?;
        let (pw, ph) = def.period();
        if self.pattern_depth >= MAX_PATTERN_DEPTH || !in_range(&[pw, ph]) || !in_range(&at.as_coeffs()) || at.determinant().abs() <= 1e-12 {
            return None;
        }
        let cell = Rect::new(0.0, 0.0, pw, ph);
        let tiles = def.instances_in(Affine::IDENTITY, cell);
        let mut sb = s.stream_builder();
        let mut cs = sb.surface();
        let reaches_out = tiles.iter().any(|t| t.visual_bounds().is_some_and(|b| cell.intersect(b) != b));
        let clip = reaches_out.then(|| to_path(&cs, &cell.to_path(0.1))).flatten();
        if let Some(c) = &clip {
            cs.push_clip_path(c, &krilla::paint::FillRule::NonZero);
        }
        // Pattern art is a picture of its own: it takes no part in a knockout group around the object.
        let knockout = std::mem::take(&mut self.knockout);
        self.pattern_depth += 1;
        for t in &tiles {
            self.node(&mut cs, t, cell, true);
        }
        self.pattern_depth -= 1;
        self.knockout = knockout;
        if clip.is_some() {
            cs.pop();
        }
        cs.finish();
        Some(krilla::paint::Pattern { stream: sb.finish(), transform: xf(at), width: pw as f32, height: ph as f32 }.into())
    }

    /// Draw gradient `g`, placed on `bounds`, as an image over `area`, sampled at the document's
    /// raster effects resolution where it is drawn (`area` as the current transform sizes it, so
    /// scaled type gets its pixels too): a freeform gradient's colour field (at most
    /// [`MAX_FIELD_PX`] a side), or a linear or radial gradient (at most [`MAX_GRADIENT_PX`] a side),
    /// with dither's noise when it paints a fill (`fill`).
    fn field_image(&mut self, s: &mut Surface, g: &vectorcraft_color::GradientPaint, bounds: Rect, area: Rect, fill: bool) {
        use vectorcraft_color::freeform::{painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let area = area.abs();
        // The area's sides in points on the page.
        let [m0, m1, m2, m3, _, _] = ctm(s).as_coeffs();
        let (w, h) = (area.width() * m0.hypot(m1), area.height() * m2.hypot(m3));
        let long = w.max(h);
        if !(long > 1e-9 && long.is_finite()) {
            return;
        }
        let freeform = g.gradient.kind == GradientKind::Freeform;
        let max = if freeform { MAX_FIELD_PX } else { MAX_GRADIENT_PX };
        let k = (self.doc.raster_effects_ppi / 72.0).min(max / long);
        let along = |len: f64| (len * k).ceil().clamp(1.0, max) as u16;
        let (cols, rows) = (along(w), along(h));
        let rgba: Vec<u8> = if freeform {
            let field = g.freeform_on(b).field_with(spread_scale(b), &|c| c.to_rgb());
            field.grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect()
        } else {
            self.warn("linear and radial gradients drawn as pixels are exported as images");
            g.pixels(b, area, cols, rows, fill, &|c| c.to_rgb()).flatten().collect()
        };
        let pixels = if self.convert { self.out.pixels() } else { None };
        let (Some(img), Some(size)) = (
            crate::images::from_rgba_in(rgba, cols.into(), rows.into(), self.interpolate, pixels),
            Size::from_wh(area.width() as f32, area.height() as f32),
        ) else {
            return;
        };
        s.push_transform(&xf(Affine::translate(area.origin().to_vec2())));
        s.draw_image(img, size);
        s.pop();
    }

    /// Paint stroke `st` of the shape `bp` (`path`), whose geometric bounds `bounds` place its
    /// unplaced gradients.
    #[allow(clippy::too_many_arguments)]
    fn stroke(&mut self, s: &mut Surface, bp: &BezPath, path: &Path, r: FillRule, st: &StrokeLayer, page: Rect, bounds: Rect) {
        if !stroke::is_plain(st) && self.brush_art(s, bp, st, page) {
            return;
        }
        if self.area_paint(&st.paint, false) && st.path_gradient().is_none() {
            // The area the stroke paints (dashes, caps, joins, profile, arrowheads, alignment), as
            // Outline Stroke makes it, painted with the pattern's tiles or the gradient's image (a
            // gradient along or across the stroke is written as slices below).
            let region = stroke::outline_region(&vectorcraft_geom::PathData::from_bezpath(bp), r, st).to_bezpath();
            if let Some(clip) = to_path(s, &region) {
                let area = region.bounding_box();
                self.area(s, &clip, krilla::paint::FillRule::NonZero, &st.paint, (st.opacity, st.blend), st.paint_bounds(bounds), area, false);
            }
            return;
        }
        let Some(paint) = self.paint(s, &st.paint, st.paint_bounds(bounds)) else { return };
        let w = stroke::for_writer(bp, st);
        // A stroke whose width, caps or miter spikes reach beyond [`MAX_COORD`] is left out.
        if !writable(s, Affine::IDENTITY, w.reach(st, bp.control_box()).inflate(1.0, 1.0)) {
            return;
        }
        let mut pushes = 0;
        if st.blend != BlendMode::Normal {
            s.push_blend_mode(blend(st.blend));
            pushes += 1;
        }
        match w.side {
            Some(StrokeAlign::Inside) => {
                s.push_clip_path(path, &rule(r));
                pushes += 1;
            }
            Some(StrokeAlign::Outside) => {
                // Clip to everything outside the path: a frame around all the stroke reaches
                // (miter spikes included) plus the path, even-odd.
                let mut outside = w.reach(st, bounds).inflate(1.0, 1.0).to_path(0.1);
                outside.extend(bp.iter());
                if let Some(p) = to_path(s, &outside) {
                    s.push_clip_path(&p, &krilla::paint::FillRule::EvenOdd);
                    pushes += 1;
                }
            }
            _ => {}
        }
        match &w.shape {
            WrittenShape::Stroke { width } => {
                let dash = st.dash.as_ref().filter(|d| d.is_dashed()).map(|d| {
                    let mut pat: Vec<f32> = d.pattern.iter().map(|v| *v as f32).collect();
                    if pat.len() % 2 == 1 {
                        pat.extend(pat.clone());
                    }
                    // An offset beyond [`MAX_COORD`] is written as its remainder after whole
                    // periods of the pattern.
                    let period: f64 = pat.iter().copied().map(f64::from).sum();
                    let offset = if in_range(&[d.offset]) { d.offset } else { d.offset.rem_euclid(period) };
                    StrokeDash { array: pat, offset: if offset.is_finite() { offset as f32 } else { 0.0 } }
                });
                s.set_fill(None);
                s.set_stroke(Some(Stroke {
                    paint,
                    width: *width as f32,
                    miter_limit: st.miter_limit.max(1.0) as f32,
                    line_cap: match st.cap {
                        LineCap::Butt => krilla::paint::LineCap::Butt,
                        LineCap::Round => krilla::paint::LineCap::Round,
                        LineCap::Square => krilla::paint::LineCap::Square,
                    },
                    line_join: match st.join {
                        LineJoin::Miter => krilla::paint::LineJoin::Miter,
                        LineJoin::Round => krilla::paint::LineJoin::Round,
                        LineJoin::Bevel => krilla::paint::LineJoin::Bevel,
                    },
                    opacity: norm(st.opacity),
                    dash,
                }));
                s.draw_path(path);
                s.set_stroke(None);
            }
            WrittenShape::Fill(outlines) if st.path_gradient().is_some() => {
                // A gradient along or across the stroke: slices clipped to its outlines, under
                // the stroke's opacity as a group.
                if let Some(ws) = stroke::written_slices(bp, r, st, outlines)
                    && let Some(clip) = to_path(s, &ws.clip)
                {
                    self.warn("gradients along or across strokes are exported as slices of linear gradients");
                    if st.opacity < 1.0 {
                        s.push_opacity(norm(st.opacity));
                        pushes += 1;
                    }
                    s.push_clip_path(&clip, &krilla::paint::FillRule::NonZero);
                    s.set_stroke(None);
                    for (shape, paint) in &ws.slices {
                        let b = shape.bounding_box();
                        if let (Some(paint), Some(p)) = (self.paint(s, paint, b), to_path(s, shape)) {
                            s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
                            s.draw_path(&p);
                        }
                    }
                    s.set_fill(None);
                    s.pop();
                }
            }
            WrittenShape::Fill(outlines) => {
                // The line and its arrowheads overlap: they take the opacity once, as a group.
                let grouped = outlines.len() > 1 && st.opacity < 1.0;
                if grouped {
                    s.push_opacity(norm(st.opacity));
                    pushes += 1;
                }
                let opacity = if grouped { NormalizedF32::ONE } else { norm(st.opacity) };
                s.set_stroke(None);
                s.set_fill(Some(Fill { paint, opacity, rule: krilla::paint::FillRule::NonZero }));
                for outline in outlines {
                    if let Some(p) = to_path(s, outline) {
                        s.draw_path(&p);
                    }
                }
                s.set_fill(None);
            }
        }
        for _ in 0..pushes {
            s.pop();
        }
    }

    /// Paint stroke `st` of the shape `bp` with its brush art (the stroke's opacity and blend mode
    /// over all of it). False when it has no known brush.
    fn brush_art(&mut self, s: &mut Surface, bp: &BezPath, st: &StrokeLayer, page: Rect) -> bool {
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) else { return false };
        let art = vectorcraft_brush::stroke_pieces(doc, b, bp, st);
        let mut pushes = 0;
        if st.blend != BlendMode::Normal {
            s.push_blend_mode(blend(st.blend));
            pushes += 1;
        }
        if st.opacity < 1.0 {
            s.push_opacity(norm(st.opacity));
            pushes += 1;
        }
        for piece in &art {
            self.node(s, piece, page, false);
        }
        for _ in 0..pushes {
            s.pop();
        }
        true
    }

    ///
    /// Inline graphics ([`vectorcraft_doc::TextRun::inline`]) are drawn as their symbols' art
    /// (vector paths, like symbol instances) where the layout placed them; their characters
    /// write no glyph.
    fn text(&mut self, s: &mut Surface, n: &Node, t: &TextObject, page: Rect) {
        let doc = self.doc;
        let resolved = doc.inline_resolved(t);
        let t = &*resolved;
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let tb = t.xf.transform_rect_bbox(layout.bounds);
        // The object's own fills and strokes paint the whole outline: those below the Characters
        // row under the characters, the others over them.
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = layout.to_bezpath();
            for (_, bar) in vectorcraft_text::decorations(&layout, vectorcraft_text::FontDb::global(), t) {
                all.extend(bar.iter());
            }
            all.apply_affine(t.xf);
            to_path(s, &all).map(|p| (all, p))
        });
        let all = all.flatten();
        if let Some((bp, path)) = &all {
            self.text_items(s, below, bp, path, page, tb);
        }
        // Real text: the characters' fills as text in embedded fonts. The type's transform is
        // pushed for each fill and stroke of the characters, inside its form when the fill or
        // stroke overprints.
        let text_space = xf(t.xf);
        // Ruby can't be real text in one run (it isn't characters of the base text): the type's
        // characters go out as outlines where ruby is present, ruby included.
        let plain = (!self.outline_text && t.ruby.is_empty()).then(|| t.plain_text());
        let bars = vectorcraft_text::decorations(&layout, vectorcraft_text::FontDb::global(), t);
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i).chain(layout.ruby.iter().filter(|g| g.run == i)) {
                bp.extend(g.outline.iter());
            }
            // Underline and strikethrough bars are paths, with real text too (#847).
            let mut run_bars = BezPath::new();
            for (_, bar) in bars.iter().filter(|(r, _)| *r == i) {
                run_bars.extend(bar.iter());
            }
            bp.extend(run_bars.iter());
            let fill = &run.style.fill;
            // Colour bitmap glyphs (emoji) real text doesn't write: all of them, or those it leaves.
            let mut images: Option<Vec<usize>> = None;
            self.overprinting(s, run.style.overprint_fill, fill, |ex, s| {
                s.push_transform(&text_space);
                // The outlines left to fill as paths: all of them, or those real text leaves.
                let outlines = match plain.as_deref().filter(|_| !fill.is_none() && !ex.area_paint(fill, true)) {
                    Some(text) => {
                        let (mut left, unwritten) = ex.glyph_text(s, &layout, i, text, fill, true);
                        images = Some(unwritten);
                        left.extend(run_bars.iter());
                        Cow::Owned(left)
                    }
                    None => Cow::Borrowed(&bp),
                };
                let opaque = (1.0, BlendMode::Normal);
                if let Some(path) = to_path(s, &outlines)
                    && !ex.area(s, &path, krilla::paint::FillRule::NonZero, fill, opaque, layout.bounds, outlines.bounding_box(), true)
                    && let Some(paint) = ex.paint(s, fill, layout.bounds)
                {
                    s.set_stroke(None);
                    s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
                    s.draw_path(&path);
                }
                s.pop();
            });
            // They paint themselves, whatever the fill.
            let images = images.unwrap_or_else(|| layout.glyphs.iter().enumerate().filter(|(_, g)| g.run == i).map(|(gi, _)| gi).collect());
            s.push_transform(&text_space);
            for g in images.iter().filter_map(|&gi| layout.glyphs.get(gi)) {
                self.glyph_image(s, g);
                // COLR glyphs: their layers, as art.
                if let Some(art) = g.placed_colour(Affine::IDENTITY) {
                    self.node(s, &art, page, true);
                }
            }
            s.pop();
            // Characters with neither fill nor stroke (a scanned page's recognized text), or whose
            // fill is drawn as art (a dithered gradient's image, a pattern: outlines, not text),
            // are still text: written invisible (rendering mode 3), so they can still be selected
            // and searched.
            let unpainted = fill.is_none() && !run.style.has_stroke();
            if let Some(text) = plain.as_deref().filter(|_| unpainted || self.area_paint(fill, true)) {
                self.invisible = true;
                crate::forms::form(s, Mark::Invisible, |s| {
                    s.push_transform(&text_space);
                    // The paint only lets the writer write the text; the glyphs it can't write
                    // as text draw nothing, and colour bitmap glyphs are drawn already.
                    self.glyph_text(s, &layout, i, text, &Paint::solid(Color::gray(0.0)), false);
                    s.pop();
                });
            }
            s.push_transform(&text_space);
            let path = run.style.has_stroke().then(|| to_path(s, &bp)).flatten();
            s.pop();
            if let Some(path) = path {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                let st = run.style.stroke_layer();
                self.overprinting(s, st.overprint, &st.paint, |ex, s| {
                    s.push_transform(&text_space);
                    ex.stroke(s, &bp, &path, FillRule::NonZero, &st, page, layout.bounds);
                    s.pop();
                });
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
        if self.inline_depth < MAX_INLINE_DEPTH {
            self.inline_depth += 1;
            for ig in &layout.inlines {
                let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
                let Some(sym) = doc.symbols.iter().find(|x| x.name == art.symbol) else { continue };
                let mut node = (*sym.art).clone();
                // Strokes scale with the art, as on the canvas.
                node.transform(t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol), true);
                self.node(s, &node, page, true);
            }
            self.inline_depth -= 1;
        }
        if let Some((bp, path)) = &all {
            self.text_items(s, above, bp, path, page, tb);
        }
    }

    /// Some of a type object's own fills and strokes (`items`) on its glyph outlines `bp` (`path`;
    /// `tb`: their bounds). Strokes take every stroke option, as on paths.
    #[allow(clippy::too_many_arguments)]
    fn text_items(&mut self, s: &mut Surface, items: &[AppearanceItem], bp: &BezPath, path: &Path, page: Rect, tb: Rect) {
        for item in items {
            match item {
                AppearanceItem::Fill(fl) if fl.visible => self.overprinting(s, fl.overprint, &fl.paint, |ex, s| {
                    if !ex.area(s, path, krilla::paint::FillRule::NonZero, &fl.paint, (fl.opacity, fl.blend), tb, bp.bounding_box(), true)
                        && let Some(paint) = ex.paint(s, &fl.paint, tb)
                    {
                        s.set_stroke(None);
                        s.set_fill(Some(Fill { paint, opacity: norm(fl.opacity), rule: krilla::paint::FillRule::NonZero }));
                        s.draw_path(path);
                    }
                }),
                AppearanceItem::Stroke(st) if st.visible && st.width > 0.0 => {
                    self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, bp, path, FillRule::NonZero, st, page, tb))
                }
                _ => {}
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
    }

    /// Fill the glyphs of run `run` of `layout` with `fill` as text in their fonts, embedded (in
    /// text space; `text` is the plain text, whose characters the glyphs stand for, which makes
    /// the text selectable and searchable) → the outlines of the glyphs left to draw as paths:
    /// those of fonts that can't be embedded, and missing glyphs. Glyphs sharing a font, a frame
    /// (size, scale, rotation) and a baseline are written together. Colour glyphs (bitmap emoji,
    /// COLR, SVG), with `images`, are written as text too, which draws their images or layers; the
    /// indices of those it can't write or draw (SVG glyphs) are returned with the outlines. The
    /// hyphen the layout adds where it breaks a line inside a word ([`break_hyphen`]) is written
    /// as the character its glyph stands for, on its own after the glyphs before it.
    fn glyph_text(&mut self, s: &mut Surface, layout: &TextLayout, run: usize, text: &str, fill: &Paint, images: bool) -> (BezPath, Vec<usize>) {
        let (mut rest, mut unwritten) = (BezPath::new(), vec![]);
        let mut line: Option<GlyphLine> = None;
        // The characters the glyphs stand for: the plain text, then, when the run has glyphs that
        // stand for no character of it, the characters a line-break hyphen can stand for.
        let written: Cow<str> =
            if layout.glyphs.iter().any(|g| g.run == run && g.len == 0) { Cow::Owned(format!("{text}{BREAK_HYPHENS}")) } else { Cow::Borrowed(text) };
        for (gi, g) in layout.glyphs.iter().enumerate().filter(|(_, g)| g.run == run) {
            let coloured = g.image.is_some() || g.colour.is_some();
            if coloured && !images {
                continue;
            }
            // A list marker's glyphs stand for no character of the text either; they are not
            // line-break hyphens, whatever the marker.
            let hyphen = if layout.marker_of(gi).is_some() { None } else { break_hyphen(g) };
            let range = hyphen.clone().map_or(g.byte..g.byte + g.len, |h| text.len() + h.start..text.len() + h.end);
            let ch = written.get(range.clone()).and_then(|t| t.chars().next());
            // Spaces are written too (text extraction needs them); control characters and soft
            // hyphens draw nothing.
            let blank = g.outline.elements().is_empty() && !coloured;
            if blank && !ch.is_some_and(|c| c.is_whitespace() && !c.is_control()) {
                continue;
            }
            // The glyph at the font size of its vertical scale: `frame` maps that size's glyph
            // space (y down, krilla's) to text space.
            let [a, b, c, d, ..] = g.xf.as_coeffs();
            let (sx, sy) = (a.hypot(b), c.hypot(d));
            let font = if g.gid != 0 && ch.is_some() { self.font(g.font_id) } else { None };
            let det = g.xf.determinant();
            let Some((font, upem)) = font.filter(|(_, upem)| det.is_finite() && det.abs() > 1e-9 && (1e-3..=1e5).contains(&(upem * sy))) else {
                rest.extend(g.outline.iter());
                if coloured {
                    unwritten.push(gi);
                }
                continue;
            };
            // SVG glyphs are written as text that draws nothing (see the serialize settings): their
            // art is drawn, or else their outline.
            if is_svg_glyph(g) {
                if coloured {
                    unwritten.push(gi);
                } else {
                    rest.extend(g.outline.iter());
                }
            }
            let size = upem * sy;
            let frame = g.xf * Affine::scale(1.0 / sy);
            // Its pen position along the frame's baseline, when it continues the line (a
            // line-break hyphen never does).
            let x = line.as_ref().filter(|l| hyphen.is_none() && l.face == g.font_id && (l.size - size).abs() < 1e-6).and_then(|l| {
                let [a, b, c, d, e, f] = (l.frame.inverse() * frame).as_coeffs();
                let same = (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6 && f.abs() < 1e-4;
                same.then_some(e)
            });
            let x = match x {
                Some(x) => x,
                None => {
                    if let Some(l) = line.take() {
                        self.draw_glyph_line(s, l, &written, fill, layout.bounds);
                    }
                    line = Some(GlyphLine { face: g.font_id, font, size, frame, glyphs: vec![] });
                    0.0
                }
            };
            // The glyph's advance in frame units (the last one's ends the line).
            let advance = g.advance * sy / sx;
            if let Some(l) = line.as_mut() {
                l.glyphs.push((g.gid, x, advance, range));
            }
            // A line-break hyphen is written on its own.
            if hyphen.is_some()
                && let Some(l) = line.take()
            {
                self.draw_glyph_line(s, l, &written, fill, layout.bounds);
            }
        }
        if let Some(l) = line {
            self.draw_glyph_line(s, l, &written, fill, layout.bounds);
        }
        (rest, unwritten)
    }

    /// Draw colour bitmap glyph `g`'s image (on `s` in text space), its PNG written once.
    fn glyph_image(&mut self, s: &mut Surface, g: &vectorcraft_text::PositionedGlyph) {
        let Some((image, to_text)) = g.placed_image() else { return };
        if !writable(s, to_text, image.rect()) {
            return;
        }
        let cache = format!("\u{0}glyph\u{0}{}", image.blob.content_key());
        let img = match self.images.get(&cache) {
            Some(img) => img.clone(),
            None => {
                let img = embed(&image.blob, self.interpolate, self.sixteen_bit);
                self.images.insert(cache, img.clone());
                img
            }
        };
        let (Some(img), Some(size)) = (img, Size::from_wh(image.width as f32, image.height as f32)) else { return };
        s.push_transform(&xf(to_text));
        s.draw_image(img, size);
        s.pop();
    }

    /// Draw glyphs `line` (of `text`), filled with `fill` (whose space is text space, where
    /// `bounds` is). A line that would be written with a pen position, size or spacing (in
    /// thousandths of an em) beyond [`MAX_COORD`] is left out.
    fn draw_glyph_line(&mut self, s: &mut Surface, line: GlyphLine, text: &str, fill: &Paint, bounds: Rect) {
        let size = line.size;
        let glyphs: Vec<KrillaGlyph> = line
            .glyphs
            .iter()
            .enumerate()
            .map(|(i, (gid, x, advance, range))| {
                let next = line.glyphs.get(i + 1).map_or(x + advance, |n| n.1);
                KrillaGlyph::new(GlyphId::new(*gid), ((next - x) / size) as f32, 0.0, 0.0, 0.0, range.clone(), None)
            })
            .collect();
        let pens =
            line.glyphs.iter().fold(Rect::new(0.0, -size, 0.0, size), |r, (_, x, advance, _)| r.union_pt((*x, 0.0)).union_pt((x + advance, 0.0)));
        let spacing: Vec<f64> = glyphs.iter().map(|g| f64::from(g.x_advance(1.0)) * 1000.0).collect();
        if !(writable(s, line.frame, pens) && in_range(&spacing)) {
            return;
        }
        s.push_transform(&xf(line.frame));
        if let Some(paint) = self.paint_in(s, fill, bounds, line.frame.inverse()) {
            s.set_stroke(None);
            s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
            s.draw_glyphs(krilla::geom::Point::from_xy(0.0, 0.0), &glyphs, line.font, text, size as f32, false);
        }
        s.pop();
    }

    /// The font of face `id` for real text and its units per em; `None` (with a warning) when its
    /// licence doesn't allow embedding it as a subset, or it can't be read.
    fn font(&mut self, id: u32) -> Option<(krilla::text::Font, f64)> {
        if let Some(f) = self.fonts.get(&id) {
            return f.clone();
        }
        let Some(face) = vectorcraft_text::FontDb::global().face_by_id(id) else {
            self.fonts.insert(id, None);
            return None;
        };
        let name = format!("{} {}", face.family, face.style);
        let font = match face.embedding() {
            Embedding::Subset => {
                // A named instance of a variable font is embedded instanced (its own outlines and
                // widths, not the default instance's).
                match write::font(face.file_data(), face.face_index(), Some(face.variations())) {
                    Ok(font) => Some((font, face.units_per_em())),
                    Err(error) => {
                        self.warn(format!("the font “{name}” can't be embedded ({error}): its text is exported as outlines"));
                        None
                    }
                }
            }
            Embedding::Whole => {
                self.warn(format!(
                    "the font “{name}” may only be embedded whole, and fonts are embedded as subsets: its text is exported as outlines"
                ));
                None
            }
            Embedding::Forbidden => {
                self.warn(format!("the licence of the font “{name}” doesn't allow embedding: its text is exported as outlines"));
                None
            }
        };
        self.fonts.insert(id, font.clone());
        font
    }

    /// Image `im` placed `size` points wide and high, resampled and compressed as the Compression
    /// settings say ([`crate::images::recode`], [`crate::images::recode_cmyk`] for CMYK images);
    /// cached by key (and size when images are resampled). `None`, with [`UNDECODED`] among the
    /// warnings, for an image that is missing, can't be decoded or is in [`Self::left_out`].
    fn load_image(&mut self, im: &vectorcraft_doc::ImageObject, size: (f64, f64)) -> Option<Image> {
        let key = im.key.as_str();
        let c = self.compression;
        let sized = [c.color.downsample, c.gray.downsample, c.mono.downsample].iter().any(|d| *d != crate::Downsample::None);
        let cache = if sized { format!("{key}\u{0}{:.3}x{:.3}", size.0, size.1) } else { key.to_string() };
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let interpolate = self.interpolate;
        let doc = self.doc;
        let out = self.out.clone();
        let img = doc.image_blob(im).filter(|_| !self.left_out.contains(key)).and_then(|blob| {
            let cmyk_image = blob.is_cmyk();
            if !cmyk_image && self.convert && doc.color_mode == vectorcraft_doc::ColorMode::Cmyk {
                self.rgb_in_cmyk.images.insert(key.to_string());
            }
            let r = if cmyk_image {
                crate::images::recode_cmyk(blob, size, c, interpolate, out.cmyk_pixels(), None)
            } else {
                crate::images::recode(&blob.bytes, size, c, interpolate, out.pixels())
            };
            if let Some(w) = r.warning {
                self.warn(w);
            }
            r.image.or_else(|| embed(blob, interpolate, self.sixteen_bit))
        });
        if img.is_none() {
            self.warn(UNDECODED);
        }
        self.images.insert(cache, img.clone());
        img
    }

    /// CMYK image `im` with the alpha of its opacity mask `m`, when the mask is a greyscale image
    /// on the image's own pixel grid (an image's soft mask as it opens): the alpha is written as the
    /// image's soft mask, as an RGBA image's is, rather than as a luminosity soft mask over it,
    /// through which CoreGraphics draws CMYK images on black boxes (#2159). `None` for other
    /// images and masks; cached like [`Self::load_image`].
    fn masked_image(&mut self, im: &vectorcraft_doc::ImageObject, m: &vectorcraft_doc::OpacityMask) -> Option<Image> {
        let art = &m.art;
        let NodeKind::Image(mi) = &art.kind else { return None };
        let plain =
            art.opacity >= 1.0 && art.blend == BlendMode::Normal && art.extras().mask.is_none() && !art.appearance.effects.iter().any(|e| e.visible);
        if m.invert || !plain || (mi.xf, mi.width, mi.height) != (im.xf, im.width, im.height) {
            return None;
        }
        let size = placed_size(im);
        let cache = format!("{}\u{0}{}\u{0}{}\u{0}{:.3}x{:.3}", im.key, mi.key, m.clip, size.0, size.1);
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let doc = self.doc;
        let img = doc.image_blob(im).filter(|b| b.is_cmyk()).and_then(|blob| {
            let alpha = crate::images::mask_alpha(doc.image_blob(mi)?, im.width, im.height, m.clip)?;
            let r = crate::images::recode_cmyk(blob, size, self.compression, self.interpolate, self.out.cmyk_pixels(), Some(alpha));
            if let (Some(w), Some(_)) = (r.warning, &r.image) {
                self.warn(w);
            }
            r.image
        });
        self.images.insert(cache, img.clone());
        img
    }

    /// Draw image `im`: `img` when given (its pixels already written), else as it is loaded.
    fn image(&mut self, s: &mut Surface, im: &vectorcraft_doc::ImageObject, img: Option<Image>) {
        // An image placed or sized beyond [`MAX_COORD`] is left out.
        if !writable(s, im.xf, Rect::new(0.0, 0.0, im.width.max(1).into(), im.height.max(1).into())) {
            return;
        }
        let Some(img) = img.or_else(|| self.load_image(im, placed_size(im))) else { return };
        let Some(size) = Size::from_wh(im.width.max(1) as f32, im.height.max(1) as f32) else { return };
        s.push_transform(&xf(im.xf));
        s.draw_image(img, size);
        s.pop();
    }
}

/// The size image `im` is placed at: its pixel grid's sides through its transform.
fn placed_size(im: &vectorcraft_doc::ImageObject) -> (f64, f64) {
    let [a, b, c, d, ..] = im.xf.as_coeffs();
    (im.width as f64 * a.hypot(b), im.height as f64 * c.hypot(d))
}

/// Does PNG `bytes` have 16 bits a sample: the bit depth of its IHDR chunk, which follows the
/// signature?
fn png_16_bit(bytes: &[u8]) -> bool {
    bytes.get(12..16) == Some(b"IHDR".as_slice()) && bytes.get(24) == Some(&16)
}

/// PNG `bytes` at 8 bits a sample: decoded, and encoded again as gray or RGB, with alpha when the
/// source has it, and with the source's ICC profile. `None` when it can't be decoded.
fn png_8_bit(bytes: &[u8]) -> Option<Vec<u8>> {
    use image::{ColorType, DynamicImage, ImageDecoder, ImageEncoder};
    let mut decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let icc = decoder.icc_profile().ok().flatten();
    let img = DynamicImage::from_decoder(decoder).ok()?;
    let img = match img.color() {
        ColorType::L8 | ColorType::L16 => DynamicImage::ImageLuma8(img.into_luma8()),
        ColorType::La8 | ColorType::La16 => DynamicImage::ImageLumaA8(img.into_luma_alpha8()),
        ColorType::Rgb8 | ColorType::Rgb16 => DynamicImage::ImageRgb8(img.into_rgb8()),
        _ => DynamicImage::ImageRgba8(img.into_rgba8()),
    };
    let mut png = vec![];
    let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc).ok()?;
    }
    img.write_with_encoder(encoder).ok()?;
    Some(png)
}

/// Image `blob` as it is: PNG, JPEG, GIF and WebP through the PDF writer (JPEG data unchanged),
/// other formats decoded. Without `sixteen_bit` (a file before PDF 1.5), a PNG of 16 bits a
/// sample is written at 8 bits ([`png_8_bit`]).
fn embed(blob: &vectorcraft_doc::ImageBlob, interpolate: bool, sixteen_bit: bool) -> Option<Image> {
    let bytes = blob.bytes.as_ref().clone();
    let direct = match blob.mime.as_str() {
        "image/png" if !sixteen_bit && png_16_bit(&bytes) => png_8_bit(&bytes).and_then(|png| Image::from_png(png.into(), interpolate).ok()),
        "image/png" => Image::from_png(bytes.clone().into(), interpolate).ok(),
        "image/jpeg" | "image/jpg" => Image::from_jpeg(bytes.clone().into(), interpolate).ok(),
        "image/gif" => Image::from_gif(bytes.clone().into(), interpolate).ok(),
        "image/webp" => Image::from_webp(bytes.clone().into(), interpolate).ok(),
        _ => None,
    };
    direct.or_else(|| {
        let rgba = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let (w, h) = rgba.dimensions();
        Some(Image::from_rgba8(rgba.into_raw(), w, h))
    })
}

/// Is `g` drawn from its font's SVG document, which the PDF writer can't draw as a font's glyph?
fn is_svg_glyph(g: &vectorcraft_text::PositionedGlyph) -> bool {
    vectorcraft_text::FontDb::global().is_svg_glyph(g.font_id, g.gid)
}

/// The characters a line-break hyphen can stand for, as real text: '-' and U+2010 (HYPHEN).
const BREAK_HYPHENS: &str = "-\u{2010}";

/// The bytes of [`BREAK_HYPHENS`] that glyph `g` stands for when it is a hyphen the layout adds
/// where it breaks a line inside a word: a glyph that stands for no character of the text (length
/// 0) and that its face's character map gives to '-' or, failing that, to U+2010. `None` for any
/// other glyph, kashida tatweels included.
fn break_hyphen(g: &vectorcraft_text::PositionedGlyph) -> Option<std::ops::Range<usize>> {
    if g.len != 0 || g.gid == 0 {
        return None;
    }
    let face = vectorcraft_text::FontDb::global().face_by_id(g.font_id)?;
    BREAK_HYPHENS.char_indices().find(|&(_, c)| face.glyph_for(c) == g.gid).map(|(i, c)| i..i + c.len_utf8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_bounds_values_the_transform_and_values_through_it() {
        let mut pdf = krilla::Document::new();
        let mut page = pdf.start_page_with(PageSettings::new(Size::from_wh(100.0, 100.0).unwrap()));
        let mut s = page.surface();
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(writable(&s, Affine::IDENTITY, r));
        assert!(!writable(&s, Affine::IDENTITY, Rect::new(-2_147_483_649.0, 0.0, 10.0, 10.0)));
        assert!(!writable(&s, Affine::IDENTITY, Rect::new(f64::NAN, 0.0, 10.0, 10.0)));
        assert!(!writable(&s, Affine::translate((0.0, 2e9)), r));
        assert!(!writable(&s, Affine::scale(2e9), Rect::ZERO));
        // `writable` also checks the surface's own transform: with the surface moved to
        // x = MAX_COORD, a point at the origin fits and a 10 by 10 box does not.
        s.push_transform(&xf(Affine::translate((MAX_COORD, 0.0))));
        assert!(writable(&s, Affine::IDENTITY, Rect::ZERO));
        assert!(!writable(&s, Affine::IDENTITY, r));
        s.pop();
        s.finish();
        page.finish();
    }

    #[test]
    fn a_16_bit_png_at_8_bits_keeps_its_color_type_and_icc_profile() {
        use image::{ImageDecoder, ImageEncoder};
        let icc = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::GRAY).unwrap().to_vec();
        let gray = image::DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(4, 2, |x, y| image::Luma([x as u16 * 20000 + y as u16])));
        let mut png = vec![];
        let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
        encoder.set_icc_profile(icc.clone()).unwrap();
        gray.write_with_encoder(encoder).unwrap();
        assert!(png_16_bit(&png));
        let out = png_8_bit(&png).unwrap();
        assert!(!png_16_bit(&out));
        let mut decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(out.as_slice())).unwrap();
        assert_eq!(decoder.icc_profile().unwrap(), Some(icc));
        assert_eq!(decoder.color_type(), image::ColorType::L8);
        assert_eq!(image::DynamicImage::from_decoder(decoder).unwrap().to_luma8(), gray.to_luma8());
    }
}
