//! Document → PDF (krilla). Mirrors the tree walk of `vectorcraft-render`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use krilla::color::separation::{Color as SepColor, SeparationColorant, SeparationSpace};
use krilla::color::{cmyk, luma, rgb};
use krilla::configure::{Archival, ConfigurationBuilder, PdfVersion};
use krilla::geom::{Path, PathBuilder, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, LinearGradient, RadialGradient, SpreadMethod, Stop, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::{GlyphId, KrillaGlyph};
use kurbo::{PathEl, Shape, Vec2};
use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, Document, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, TextObject};
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
    let set = &opts.settings;
    set.check()?;
    // Live geometry effects export as their result; raster effects are reported below.
    let baked = vectorcraft_effects::bake_document(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    let pages = pages(doc, opts)?;

    let title = crate::pdfx::title(set.standard, opts.title.clone().unwrap_or_else(|| doc.title.clone()));
    let created_at = opts.created.or_else(vectorcraft_doc::metadata::now_unix);
    crate::pdfx::require_date(set.standard, created_at)?;
    let mut w = Writer::new(doc, set, &title, created_at, doc.color_mode == vectorcraft_doc::ColorMode::Cmyk)?;
    // Preserve Editing: the native document as an embedded file (`check` refused PDF/A with it).
    let mut warnings = set.warnings();
    let native = opts.native.as_deref().filter(|_| set.preserve_editing);
    if let Some(native) = native {
        w.pdf.embed_file(crate::editing::embedded_file(native, set.compression.compress_text, created_at.map(date_time)));
    } else if set.preserve_editing {
        warnings.push("Preserve editing needs the native document, which wasn't given: the PDF reopens as plain artwork".into());
    }

    let mut ex = Exporter::new(doc, set);
    ex.non_printing = set.include_non_printing || set.create_layers;
    ex.out = w.out.clone();
    // A knockout page group draws the layers' objects one by one, not layer by layer.
    ex.layers = set.writes_layers() && !doc.page_knockout;
    if set.writes_layers() && doc.page_knockout {
        warnings.push("PDF layers can't be written with a knockout page group: every layer is plain page content".into());
    }
    ex.overprint = set.advanced.overprint == crate::Overprint::Preserve;
    let bleed = set.bleed_of(doc);
    let marks = set.marks.printer_marks();
    for (i, boxes) in pages {
        let (m, r) = (boxes.media, boxes.bleed);
        let info = if marks.page_info { crate::marks::page_info(doc, &title, i, created_at) } else { String::new() };
        let art = crate::marks::art(doc, &marks, &boxes, bleed, &info);
        // The page is the media box, y down from its top-left corner (like ours).
        let view = Affine::translate((-m.x0, -m.y0));
        let sheet = Sheet {
            size: (m.width(), m.height()),
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
    let (layers, overprinted) = (ex.layers, ex.overprinted);
    w.absorb(ex);
    let (bytes, more) = w.finish()?;
    let bytes = crate::forms::finish(bytes, doc, set, layers, overprinted)?;
    warnings.extend(more);
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

/// A PDF being written, shared by export and print: the file's configuration and metadata, then
/// its pages ([`Self::page`]), each drawn by an [`Exporter`].
pub(crate) struct Writer {
    pub pdf: krilla::Document,
    /// How colours are written (the Output settings).
    pub out: Arc<ColorOut>,
    /// Groups blend in CMYK (a CMYK document's composite, or converting to CMYK; see
    /// `cmyk_blending`).
    cmyk: bool,
    /// Spot colours written with a Lab alternate, gathered from the exporters ([`Self::absorb`]).
    lab_spots: Vec<(String, vectorcraft_color::cms::Lab)>,
    warnings: Vec<String>,
    /// A PDF 1.3 file ([`crate::PdfSettings::pdf13`]): flat, without page groups.
    pdf13: bool,
}

/// One page as [`Writer::page`] draws it: the art placed in a drawing space ([`Self::place`]),
/// which [`Self::view`] maps onto the page.
pub(crate) struct Sheet<'a> {
    /// The page's size in points.
    pub size: (f64, f64),
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
        let mut warnings = vec![];
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
        let mut cb = ConfigurationBuilder::new().with_version(version);
        // `check` has refused the standards (and standard/version pairs) the writer can't produce.
        if set.standard == Standard::PdfA2b {
            cb = cb.with_archival_validator(Archival::A2_B);
        }
        let configuration = cb.finish().map_err(|e| PdfError::Write(format!("{e:?}")))?;
        let settings = krilla::SerializeSettings {
            compress_content_streams: set.compression.compress_text,
            no_device_cs: out.tagged,
            cmyk_profile,
            configuration,
            ..Default::default()
        };

        let mut pdf = krilla::Document::new_with(settings);
        let mut meta = Metadata::new().creator("VectorCraft".into()).producer("VectorCraft".into());
        if !title.is_empty() {
            meta = meta.title(title.to_string());
        }
        // File Info.
        let info = &doc.metadata;
        let given = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(str::to_string);
        if let Some(a) = given(&info.author) {
            meta = meta.authors(vec![a]);
        }
        if let Some(d) = given(&info.description) {
            meta = meta.description(d);
        }
        if !info.keywords.is_empty() {
            meta = meta.keywords(info.keywords.clone());
        }
        if let Some(t) = created.map(date_time) {
            meta = meta.creation_date(t);
        }
        pdf.set_metadata(meta);
        Ok(Self { pdf, cmyk: out.blends_cmyk(cmyk), out: Arc::new(out), lab_spots: vec![], warnings, pdf13: set.pdf13() })
    }

    /// Draw one page: the art of `ex`'s document placed as `sheet` says, then its marks.
    pub(crate) fn page(&mut self, ex: &mut Exporter, sheet: &Sheet) -> Result<(), PdfError> {
        let (w, h) = sheet.size;
        let size = Size::from_wh(w.max(1.0) as f32, h.max(1.0) as f32).ok_or_else(|| PdfError::BadSetting(format!("page size {w} × {h}")))?;
        let at = |b: Rect| krilla::geom::Rect::from_ltrb(b.x0 as f32, b.y0 as f32, b.x1 as f32, b.y1 as f32);
        let mut page = self.pdf.start_page_with(PageSettings::new(size).with_trim_box(at(sheet.trim)).with_bleed_box(at(sheet.bleed)));
        let mut s = page.surface();
        let paper = Rect::new(0.0, 0.0, w, h);
        if sheet.negative {
            cover(&mut s, paper, 255, 1.0);
        }
        s.push_transform(&xf(sheet.view));
        let window = sheet.window.and_then(|r| to_path(&r.to_path(0.1)));
        if let Some(clip) = &window {
            s.push_clip_path(clip, &krilla::paint::FillRule::NonZero);
        }
        let placed = sheet.place != Affine::IDENTITY;
        if placed {
            s.push_transform(&xf(sheet.place));
        }
        let r = sheet.area;
        let clip = sheet.clip.then(|| to_path(&r.to_path(0.1))).flatten();
        if let Some(clip) = &clip {
            s.push_clip_path(clip, &krilla::paint::FillRule::NonZero);
        }
        // Page Isolated Blending / Page Knockout Group: the page content is one group (the PDF
        // writer has no page group attributes; a flat PDF 1.3 file has none). In CMYK, transparency
        // at the top of a page is put in a non-isolated group of its own, whose blending space
        // `cmyk_blending` rewrites.
        let doc = ex.doc;
        let page_group = !self.pdf13 && (doc.page_isolate || doc.page_knockout);
        let cmyk_page_group = self.cmyk && !page_group && ex.shows_transparency();
        if page_group {
            s.push_isolated();
        } else if cmyk_page_group {
            let all = constant_mask(&mut s, r, 1.0);
            s.push_mask(all);
        }
        ex.children(&mut s, &doc.layers, r);
        if page_group || cmyk_page_group {
            s.pop();
        }
        if clip.is_some() {
            s.pop();
        }
        if placed {
            s.pop();
        }
        if let Some(art) = sheet.marks {
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

    /// Take an exporter's warnings and Lab spot colours once its pages are drawn.
    pub(crate) fn absorb(&mut self, ex: Exporter) {
        for w in ex.warnings {
            if !self.warnings.contains(&w) {
                self.warnings.push(w);
            }
        }
        for spot in ex.lab_spots {
            if !self.lab_spots.iter().any(|(n, _)| *n == spot.0) {
                self.lab_spots.push(spot);
            }
        }
    }

    /// The finished file (with the output intent and Trapped entries, and checked against its
    /// standard) and the warnings of its drawing. PDF/X-1a spot colours keep their CMYK
    /// alternates.
    pub(crate) fn finish(mut self) -> Result<(Vec<u8>, Vec<String>), PdfError> {
        let mut bytes = self.pdf.finish().map_err(|e| PdfError::Write(format!("{e:?}")))?;
        if self.cmyk {
            cmyk_blending(&mut bytes, self.out.tagged);
        }
        let lab = !self.lab_spots.is_empty() && !self.out.standard.cmyk_only();
        let bytes = if lab { crate::lab_spot::lab_alternates(bytes, &self.lab_spots) } else { bytes };
        let (bytes, more) = self.out.write_catalog(bytes)?;
        self.warnings.extend(more);
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
    /// Inline graphics being drawn inside inline graphics (text in a symbol shown inline in
    /// text…): deeper ones are left out.
    inline_depth: u32,
}

/// How deep inline graphics nest before they are left out.
const MAX_INLINE_DEPTH: u32 = 4;

impl<'a> Exporter<'a> {
    /// An exporter of `doc` writing images as `set` says; layers whose Print option is off are
    /// left out.
    pub(crate) fn new(doc: &'a Document, set: &'a crate::PdfSettings) -> Self {
        Self {
            doc,
            warnings: vec![],
            images: HashMap::new(),
            brushes: None,
            knockout: doc.page_knockout,
            lab_spots: vec![],
            interpolate: set.standard != Standard::PdfA2b,
            compression: &set.compression,
            non_printing: false,
            intent: vectorcraft_color::cms::active().settings().intent,
            transparent: None,
            out: Arc::default(),
            convert: true,
            outline_text: set.advanced.outline_text,
            fonts: HashMap::new(),
            layers: false,
            layer_index: crate::forms::pdf_layers(doc).iter().enumerate().map(|(i, l)| (address(l.node), i)).collect(),
            in_layer: 0,
            overprint: false,
            overprinted: false,
            inline_depth: 0,
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

fn xf(a: Affine) -> Transform {
    let c = a.as_coeffs();
    Transform::from_row(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32)
}

fn to_path(bp: &BezPath) -> Option<Path> {
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
    let Some(p) = to_path(&page.inflate(1.0, 1.0).to_path(0.1)) else { return };
    s.set_stroke(None);
    s.set_fill(Some(Fill { paint: rgb::Color::new(level, level, level).into(), opacity: norm(opacity), rule: krilla::paint::FillRule::NonZero }));
    s.draw_path(&p);
}

/// An alpha mask of constant `alpha` over the page. A group drawn through it is a transparency
/// group of that opacity that, unlike the writer's opacity groups, is not isolated.
fn constant_mask(s: &mut Surface, page: Rect, alpha: f32) -> krilla::mask::Mask {
    let mut sb = s.stream_builder();
    let mut ms = sb.surface();
    cover(&mut ms, page, 0, alpha);
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
    /// active colour settings, so the file carries press values.
    fn col(&mut self, c: &Color) -> krilla::color::Color {
        let cmyk_doc = self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk;
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

    /// Convert a paint. `bounds` resolves unset gradient geometry (like the renderer).
    fn paint(&mut self, p: &Paint, bounds: Rect) -> Option<krilla::paint::Paint> {
        self.paint_in(p, bounds, Affine::IDENTITY)
    }

    /// [`Self::paint`] drawn in another space: `space` maps the paint's space (where `bounds`
    /// is) to the one drawn in.
    fn paint_in(&mut self, p: &Paint, bounds: Rect, space: Affine) -> Option<krilla::paint::Paint> {
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
            .filter(|c| c.visual_bounds().is_some_and(|b| rects_overlap(b.inflate(1.0, 1.0), page)))
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
        let shape = if c.knockout_shape { c.clone() } else { Node { opacity: 1.0, mask: None, ..c.clone() } };
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
        match n.visual_bounds() {
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
        if let Some(m) = n.mask.as_deref()
            && !m.disabled
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
                    && let Some((p, r)) = vectorcraft_effects::clip_outline(clip).and_then(|(bp, r)| to_path(&bp).map(|p| (p, r)))
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
            NodeKind::Image(im) => self.image(s, im),
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = self.doc.symbols.iter().find(|x| &x.name == symbol) {
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
            | NodeKind::CompoundShape { .. } => {
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
        let Some(path) = to_path(bp) else { return };
        let bounds = bp.bounding_box();
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    self.warn_raster(&fl.effects, || "fills".into());
                    self.overprinting(s, fl.overprint, &fl.paint, |ex, s| {
                        if ex.area(s, &path, rule(r), &fl.paint, (fl.opacity, fl.blend), bounds, bounds) {
                            return;
                        }
                        let Some(paint) = ex.paint(&fl.paint, bounds) else { return };
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

    /// Is `p` a paint PDF has no paint for, which [`Self::area`] draws as art: a pattern swatch or
    /// a freeform gradient?
    fn area_paint(&self, p: &Paint) -> bool {
        match p {
            Paint::Pattern { pattern, .. } => self.doc.pattern(pattern).is_some(),
            Paint::Gradient(g) => g.gradient.kind == GradientKind::Freeform,
            _ => false,
        }
    }

    /// Paint the area `clip` (filled by `rule`; bounds `area`) with `p` at `transparency` (opacity,
    /// blend mode) when it is an [area paint](Self::area_paint): a pattern's tile instances
    /// covering it, or an image of a freeform gradient placed on `bounds` (the painted object's
    /// box, as on the canvas), clipped to it. False (nothing drawn) for other paints.
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
    ) -> bool {
        if !self.area_paint(p) {
            return false;
        }
        let (opacity, mode) = transparency;
        let mut pushes = 0;
        if mode != BlendMode::Normal {
            s.push_blend_mode(blend(mode));
            pushes += 1;
        }
        s.push_clip_path(clip, &rule);
        pushes += 1;
        if opacity < 1.0 {
            s.push_opacity(norm(opacity));
            pushes += 1;
        }
        let doc = self.doc;
        match p {
            Paint::Pattern { pattern, xf } => {
                for inst in doc.pattern(pattern).map(|def| def.instances_in(*xf, area)).unwrap_or_default() {
                    self.node(s, &inst, area, true);
                }
            }
            Paint::Gradient(g) => self.field_image(s, g, bounds, area),
            _ => {}
        }
        for _ in 0..pushes {
            s.pop();
        }
        true
    }

    /// Draw freeform gradient `g`, placed on `bounds`, as an image of its colour field over `area`,
    /// sampled at the document's raster effects resolution (at most [`MAX_FIELD_PX`] a side).
    fn field_image(&mut self, s: &mut Surface, g: &vectorcraft_color::GradientPaint, bounds: Rect, area: Rect) {
        use vectorcraft_color::freeform::{painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let area = area.abs();
        let long = area.width().max(area.height());
        if !(long > 1e-9 && long.is_finite()) {
            return;
        }
        let k = (self.doc.raster_effects_ppi / 72.0).min(MAX_FIELD_PX / long);
        let along = |len: f64| (len * k).ceil().clamp(1.0, MAX_FIELD_PX) as u16;
        let (cols, rows) = (along(area.width()), along(area.height()));
        let field = g.freeform_on(b).field_with(spread_scale(b), &|c| c.to_rgb());
        let rgba: Vec<u8> = field.grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect();
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
        if self.area_paint(&st.paint) {
            // The area the stroke paints (dashes, caps, joins, profile, arrowheads, alignment), as
            // Outline Stroke makes it, painted with the pattern's tiles or the freeform's image.
            let region = stroke::outline_region(&vectorcraft_geom::PathData::from_bezpath(bp), r, st).to_bezpath();
            if let Some(clip) = to_path(&region) {
                let area = region.bounding_box();
                self.area(s, &clip, krilla::paint::FillRule::NonZero, &st.paint, (st.opacity, st.blend), st.paint_bounds(bounds), area);
            }
            return;
        }
        let Some(paint) = self.paint(&st.paint, st.paint_bounds(bounds)) else { return };
        let w = stroke::for_writer(bp, st);
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
                if let Some(p) = to_path(&outside) {
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
                    StrokeDash { array: pat, offset: d.offset as f32 }
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
                    && let Some(clip) = to_path(&ws.clip)
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
                        if let (Some(paint), Some(p)) = (self.paint(paint, b), to_path(shape)) {
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
                for p in outlines.iter().filter_map(to_path) {
                    s.draw_path(&p);
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
        let art = vectorcraft_brush::stroke_pieces(b, bp, st);
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
            all.apply_affine(t.xf);
            to_path(&all).map(|p| (all, p))
        });
        let all = all.flatten();
        if let Some((bp, path)) = &all {
            self.text_items(s, below, bp, path, page, tb);
        }
        s.push_transform(&xf(t.xf));
        // Real text: the characters' fills as text in embedded fonts.
        let plain = (!self.outline_text).then(|| t.plain_text());
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            let fill = &run.style.fill;
            self.overprinting(s, run.style.overprint_fill, fill, |ex, s| {
                // The outlines left to fill as paths: all of them, or those real text leaves.
                let outlines = match plain.as_deref().filter(|_| !fill.is_none() && !ex.area_paint(fill)) {
                    Some(text) => Cow::Owned(ex.glyph_text(s, &layout, i, text, fill)),
                    None => Cow::Borrowed(&bp),
                };
                let opaque = (1.0, BlendMode::Normal);
                if let Some(path) = to_path(&outlines)
                    && !ex.area(s, &path, krilla::paint::FillRule::NonZero, fill, opaque, layout.bounds, outlines.bounding_box())
                    && let Some(paint) = ex.paint(fill, layout.bounds)
                {
                    s.set_stroke(None);
                    s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
                    s.draw_path(&path);
                }
            });
            if run.style.has_stroke()
                && let Some(path) = to_path(&bp)
            {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                let st = run.style.stroke_layer();
                self.overprinting(s, st.overprint, &st.paint, |ex, s| ex.stroke(s, &bp, &path, FillRule::NonZero, &st, page, layout.bounds));
            }
        }
        s.set_fill(None);
        s.set_stroke(None);
        s.pop();
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
                    if !ex.area(s, path, krilla::paint::FillRule::NonZero, &fl.paint, (fl.opacity, fl.blend), tb, bp.bounding_box())
                        && let Some(paint) = ex.paint(&fl.paint, tb)
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
    /// (size, scale, rotation) and a baseline are written together.
    fn glyph_text(&mut self, s: &mut Surface, layout: &TextLayout, run: usize, text: &str, fill: &Paint) -> BezPath {
        let mut rest = BezPath::new();
        let mut line: Option<GlyphLine> = None;
        for g in layout.glyphs.iter().filter(|g| g.run == run) {
            let range = g.byte..g.byte + g.len;
            let ch = text.get(range.clone()).and_then(|t| t.chars().next());
            // Spaces are written too (text extraction needs them); control characters and soft
            // hyphens draw nothing.
            let blank = g.outline.elements().is_empty();
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
                continue;
            };
            let size = upem * sy;
            let frame = g.xf * Affine::scale(1.0 / sy);
            // Its pen position along the frame's baseline, when it continues the line.
            let x = line.as_ref().filter(|l| l.face == g.font_id && (l.size - size).abs() < 1e-6).and_then(|l| {
                let [a, b, c, d, e, f] = (l.frame.inverse() * frame).as_coeffs();
                let same = (a - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6 && f.abs() < 1e-4;
                same.then_some(e)
            });
            let x = match x {
                Some(x) => x,
                None => {
                    if let Some(l) = line.take() {
                        self.draw_glyph_line(s, l, text, fill, layout.bounds);
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
        }
        if let Some(l) = line {
            self.draw_glyph_line(s, l, text, fill, layout.bounds);
        }
        rest
    }

    /// Draw glyphs `line` (of `text`), filled with `fill` (whose space is text space, where
    /// `bounds` is).
    fn draw_glyph_line(&mut self, s: &mut Surface, line: GlyphLine, text: &str, fill: &Paint, bounds: Rect) {
        let Some(paint) = self.paint_in(fill, bounds, line.frame.inverse()) else { return };
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
        s.push_transform(&xf(line.frame));
        s.set_stroke(None);
        s.set_fill(Some(Fill { paint, opacity: NormalizedF32::ONE, rule: krilla::paint::FillRule::NonZero }));
        s.draw_glyphs(krilla::geom::Point::from_xy(0.0, 0.0), &glyphs, line.font, text, size as f32, false);
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
                let coords: Vec<(krilla::text::Tag, f32)> = face.variations().iter().map(|(t, v)| (krilla::text::Tag::new(t), *v)).collect();
                let font =
                    krilla::text::Font::new_variable(face.file_data().to_vec().into(), face.face_index(), &coords).map(|f| (f, face.units_per_em()));
                if font.is_none() {
                    self.warn(format!("the font “{name}” can't be embedded: its text is exported as outlines"));
                }
                font
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

    /// Image `key` placed `size` points wide and high, resampled and compressed as the Compression
    /// settings say ([`crate::images::recode`], [`crate::images::recode_cmyk`] for CMYK images);
    /// cached by key (and size when images are resampled).
    fn load_image(&mut self, key: &str, size: (f64, f64)) -> Option<Image> {
        let c = self.compression;
        let sized = [c.color.downsample, c.gray.downsample, c.mono.downsample].iter().any(|d| *d != crate::Downsample::None);
        let cache = if sized { format!("{key}\u{0}{:.3}x{:.3}", size.0, size.1) } else { key.to_string() };
        if let Some(i) = self.images.get(&cache) {
            return i.clone();
        }
        let interpolate = self.interpolate;
        let doc = self.doc;
        let out = self.out.clone();
        let img = doc.images.get(key).and_then(|blob| {
            let r = if blob.is_cmyk() {
                crate::images::recode_cmyk(blob, size, c, interpolate, out.cmyk_pixels())
            } else {
                crate::images::recode(&blob.bytes, size, c, interpolate, out.pixels())
            };
            if let Some(w) = r.warning {
                self.warn(w);
            }
            r.image.or_else(|| embed(blob, interpolate))
        });
        if img.is_none() {
            self.warn(format!("image '{key}' could not be decoded and was skipped"));
        }
        self.images.insert(cache, img.clone());
        img
    }

    fn image(&mut self, s: &mut Surface, im: &vectorcraft_doc::ImageObject) {
        // The size it is placed at: its pixel grid's sides through its transform.
        let [a, b, c, d, ..] = im.xf.as_coeffs();
        let size = (im.width as f64 * a.hypot(b), im.height as f64 * c.hypot(d));
        let Some(img) = self.load_image(&im.key, size) else { return };
        let Some(size) = Size::from_wh(im.width.max(1) as f32, im.height.max(1) as f32) else { return };
        s.push_transform(&xf(im.xf));
        s.draw_image(img, size);
        s.pop();
    }
}

/// Image `blob` as it is: PNG, JPEG, GIF and WebP through the PDF writer (JPEG data unchanged),
/// other formats decoded.
fn embed(blob: &vectorcraft_doc::ImageBlob, interpolate: bool) -> Option<Image> {
    let bytes = blob.bytes.as_ref().clone();
    let direct = match blob.mime.as_str() {
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
