//! File → Print as a print-ready PDF: each page is a sheet of the chosen paper with the art laid
//! out on it as the [`PrintSettings`] say.
//!
//! - General: copies (collated or not), reverse order, the artboards (all, a range or ignored:
//!   all the art as one page), blank artboards skipped, the paper and its orientation (or turned
//!   to each artboard's), transverse, which layers print (template layers never do), the
//!   placement on the imageable area (or where the Print Tiling tool put the pages,
//!   [`TileOrigin`]), and the scale: none, fit, custom, or tiles of the paper or of its imageable
//!   area, overlapping, a range of them.
//! - Marks and bleed as PDF export draws them ([`crate::MarkSettings`], [`crate::BleedSettings`]),
//!   around the artboard on the paper, at the paper's scale.
//! - Output: composite, or separations, one page per ink that prints ([`print_inks`]) in the
//!   grey of its coverage (overprints honoured, spot colours as process if asked), emulsion down
//!   (mirrored), negative (inverted).
//! - Colour management: the printer profile composite colours are converted to (and
//!   separations separate with), the rendering intent, and whether CMYK colours keep their
//!   numbers.
//! - Advanced: overprints in composite output preserved, discarded or simulated (as Overprint
//!   Preview shows them). Print as Bitmap and the flattener preset need the renderer and the
//!   flattener: the app applies them to the document before printing it.
//!
//! Halftone screens and flatness are left to the output device (warnings say so). [`preview`]
//! lists the pages, tiles and inks without writing the file; [`plan`] lays the job out for other
//! writers (PostScript), which can write screens and flatness themselves; [`tiling`] turns the
//! layout into the pages in document space (View → Show Print Tiling).

mod layout;
mod plates;
mod settings;
mod tiling;

use std::borrow::Cow;
use std::sync::Arc;

use kurbo::{Affine, Rect};
use serde::Serialize;
use vectorcraft_color::cms::{self, ProfileKind};
use vectorcraft_doc::marks::PrinterMarks;
use vectorcraft_doc::overprint::{clear_overprints, multiply_overprints};
use vectorcraft_doc::{ColorMode, Document, Node, NodeKind};

pub use layout::{MAX_TILES, TileGrid};
pub use plates::print_inks;
pub use settings::*;
pub use tiling::{TilingPage, tiling};

use crate::export::{Exporter, Sheet, Writer};
use crate::marks::PageBoxes;
use crate::{AdvancedSettings, ColorConversion, CompressionSettings, OutputSettings, Overprint, PdfError, PdfSettings};
use layout::Layout;
use plates::Separator;

/// Most pages of a job, copies and inks included.
pub const MAX_PAGES: usize = 2000;

/// A print job: the settings plus what one run decides.
#[derive(Clone, Debug, Default)]
pub struct PrintOptions {
    pub settings: PrintSettings,
    /// The title in the metadata and the page information; `None` = the document's title.
    pub title: Option<String>,
    /// When the job ran, Unix seconds (UTC); `None` = now (native) / none (wasm).
    pub created: Option<i64>,
    /// Content streams uncompressed (readable operators, for tests and debugging).
    pub uncompressed: bool,
    /// The job is written as PostScript, which carries halftone screens and flatness: no
    /// warnings that they are left to the device.
    pub postscript: bool,
}

/// A printed job.
#[derive(Clone, Debug)]
pub struct PrintReport {
    pub bytes: Vec<u8>,
    pub pages: usize,
    pub warnings: Vec<String>,
}

/// One page of a job, as [`preview`] lists it (one copy).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageSummary {
    /// 0-based artboard (`None`: artboards ignored).
    pub artboard: Option<usize>,
    /// 1-based tile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<usize>,
    /// The ink of a separation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ink: Option<String>,
    /// The page size in points.
    pub width: f64,
    pub height: f64,
    pub orientation: Orientation,
    /// Horizontal and vertical scale in percent.
    pub scale: [f64; 2],
    /// Document space → the page (pt, y down from its top-left corner): where the art lands,
    /// turned and mirrored as it prints.
    pub transform: [f64; 6],
    /// The part of the document this page prints (`[x0, y0, x1, y1]`, document space): the
    /// artboard with its bleed, or what of it a tile shows.
    pub area: [f64; 4],
    /// The artboard (trim box) on the page, `[x0, y0, x1, y1]`.
    pub trim: [f64; 4],
}

/// What a job prints, without printing it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintPreview {
    /// Pages in all, copies included.
    pub pages: usize,
    /// One copy's pages, in order.
    pub sheets: Vec<PageSummary>,
    /// The tiles of each artboard, when tiling.
    pub tiles: Vec<TileGrid>,
    /// The inks of a separation (empty for composite output).
    pub inks: Vec<PrintInk>,
    pub warnings: Vec<String>,
}

/// A laid-out job.
struct Job<'a> {
    /// The document as it prints: live effects applied, the layers that print.
    doc: Cow<'a, Document>,
    layouts: Vec<Layout>,
    tiles: Vec<TileGrid>,
    /// The inks of a separation (every one, printed or not).
    inks: Vec<PrintInk>,
    /// The inks that print (indices into `inks`): each a plate.
    plates: Vec<usize>,
    /// One copy's pages: (layout, plate).
    sheets: Vec<(usize, Option<usize>)>,
    warnings: Vec<String>,
}

/// Keep the layers of `nodes` that print as `which` says, shown (hidden ones print with All),
/// down through sublayers (also what the Print dialog's preview shows).
pub fn keep_layers(nodes: &mut Vec<Arc<Node>>, which: PrintLayers) {
    nodes.retain_mut(|n| {
        let NodeKind::Layer { printable, .. } = n.kind else { return true };
        let keep = match which {
            PrintLayers::VisiblePrintable => n.visible && printable,
            PrintLayers::Visible => n.visible,
            PrintLayers::All => true,
        };
        if keep {
            let layer = Arc::make_mut(n);
            layer.visible = true;
            if let Some(children) = layer.children_mut() {
                keep_layers(children, which);
            }
        }
        keep
    });
}

/// `doc` as `set` prints it: live effects applied (raster effects are rendered by the app first),
/// only the layers that print, overprints in composite output as [`PrintAdvanced::overprints`]
/// says.
pub fn printed_document<'a>(doc: &'a Document, set: &PrintSettings) -> Cow<'a, Document> {
    let mut doc = vectorcraft_effects::bake_document(doc).map_or(Cow::Borrowed(doc), Cow::Owned);
    let d = doc.to_mut();
    keep_layers(&mut d.layers, set.print_layers);
    composite_overprints(d, set);
    doc
}

/// Overprinting fills and strokes in composite output: discarded (they knock out) or simulated
/// (drawn with Multiply, as Overprint Preview shows them), after which nothing overprints any
/// more; or preserved. Separations always honour them.
fn composite_overprints(doc: &mut Document, set: &PrintSettings) {
    let how = set.advanced.overprints;
    if set.output.mode == OutputMode::Separations || how == PrintOverprints::Preserve {
        return;
    }
    let discard_white = doc.setup.discard_white_overprint;
    let symbols = doc.symbols.iter_mut().map(|s| &mut s.art);
    let nodes = doc.layers.iter_mut().chain(symbols).chain(doc.patterns.iter_mut().flat_map(|p| p.art.iter_mut()));
    for n in nodes {
        if how == PrintOverprints::Simulate {
            multiply_overprints(n, discard_white);
        }
        clear_overprints(n);
    }
}

/// How composite output writes colours: converted to the printer profile (CMYK colours keep
/// their numbers with Preserve CMYK Numbers), overprints kept unless discarded or simulated.
fn composite_pdf(set: &PrintSettings) -> (OutputSettings, AdvancedSettings) {
    let profile = set.color.profile.trim();
    let output = if profile.is_empty() {
        OutputSettings::default()
    } else {
        let conversion = if set.color.preserve_numbers { ColorConversion::PreserveNumbers } else { ColorConversion::Destination };
        OutputSettings { conversion, destination: profile.to_string(), ..Default::default() }
    };
    let overprint = if set.advanced.overprints == PrintOverprints::Preserve { Overprint::Preserve } else { Overprint::Discard };
    (output, AdvancedSettings { overprint, ..Default::default() })
}

/// What of `doc` prints on pages of its own: each artboard printed (its rect), or all the art
/// (`None`) with artboards ignored.
pub fn print_regions(doc: &Document, set: &PrintSettings) -> Result<Vec<(Option<usize>, kurbo::Rect)>, PdfError> {
    set.check()?;
    layout::regions(&printed_document(doc, set), set)
}

impl<'a> Job<'a> {
    fn new(doc: &'a Document, set: &PrintSettings, postscript: bool) -> Result<Self, PdfError> {
        set.check()?;
        let doc = printed_document(doc, set);
        let mut warnings = vec![];
        let (layouts, tiles) = layout::layout(&doc, set, &mut warnings)?;
        let separations = set.output.mode == OutputMode::Separations;
        let (inks, more) = if separations { print_inks(&doc, set) } else { Default::default() };
        warnings.extend(more);
        let plates: Vec<usize> = (0..inks.len()).filter(|i| inks[*i].print).collect();
        let printed: Vec<Option<usize>> = if separations { (0..plates.len()).map(Some).collect() } else { vec![None] };
        if printed.is_empty() {
            return Err(PdfError::BadSetting("output.inks: no ink is set to print".into()));
        }
        let sheets: Vec<(usize, Option<usize>)> = (0..layouts.len()).flat_map(|l| printed.iter().map(move |i| (l, *i))).collect();
        if sheets.len().saturating_mul(set.copies as usize) > MAX_PAGES {
            return Err(PdfError::BadSetting(format!("the job would print more than {MAX_PAGES} pages")));
        }
        if separations && doc.color_mode == ColorMode::Rgb {
            warnings.push("the document is RGB: separations convert its colours to CMYK with the colour settings".into());
        }
        if !postscript && separations && set.output.inks.iter().any(|i| i.frequency.is_some() || i.angle.is_some()) {
            warnings.push("ink frequencies and angles are not written as halftone screens yet: the output device's screens apply".into());
        }
        if separations && set.advanced.print_as_bitmap {
            warnings.push("Print as Bitmap is for composite output: separations print the art as it is".into());
        }
        if separations && cms::profile(set.color.profile.trim()).is_some_and(|p| p.kind != ProfileKind::Cmyk) {
            warnings.push("separations separate with a CMYK profile: the RGB printer profile is left out".into());
        }
        if !postscript && !set.graphics.auto_flatness {
            warnings.push("a fixed flatness is not written yet: the output device flattens curves".into());
        }
        Ok(Self { doc, layouts, tiles, inks, plates, sheets, warnings })
    }

    /// The ink of plate `plate` (`None`: composite).
    fn ink(&self, plate: Option<usize>) -> Option<&PrintInk> {
        self.inks.get(*self.plates.get(plate?)?)
    }

    /// Every page, copies included, in print order: indices into `sheets`.
    fn order(&self, set: &PrintSettings) -> Vec<usize> {
        let copies = set.copies.max(1) as usize;
        let sheets = 0..self.sheets.len();
        let mut v: Vec<usize> = if set.collate {
            (0..copies).flat_map(|_| sheets.clone()).collect()
        } else {
            sheets.flat_map(|s| std::iter::repeat_n(s, copies)).collect()
        };
        if set.reverse {
            v.reverse();
        }
        v
    }
}

/// What printing `doc` with `set` makes: pages, tiles, inks and warnings.
pub fn preview(doc: &Document, set: &PrintSettings) -> Result<PrintPreview, PdfError> {
    let job = Job::new(doc, set, false)?;
    let sheets = job
        .sheets
        .iter()
        .filter_map(|(l, plate)| {
            let l = job.layouts.get(*l)?;
            Some(PageSummary {
                artboard: l.artboard,
                tile: l.tile.map(|t| t.0 + 1),
                ink: job.ink(*plate).map(|i| i.name.clone()),
                width: l.size.0,
                height: l.size.1,
                orientation: l.orientation,
                scale: [l.scale.0 * 100.0, l.scale.1 * 100.0],
                transform: (l.view * l.place).as_coeffs(),
                area: [l.area.x0, l.area.y0, l.area.x1, l.area.y1],
                trim: [l.page_trim.x0, l.page_trim.y0, l.page_trim.x1, l.page_trim.y1],
            })
        })
        .collect();
    Ok(PrintPreview { pages: job.sheets.len() * set.copies.max(1) as usize, sheets, tiles: job.tiles, inks: job.inks, warnings: job.warnings })
}

/// One sheet of a laid-out job, ready for a writer (PDF here, PostScript in the EPS writer).
#[derive(Clone, Debug)]
pub struct PlannedSheet {
    /// The page size in points.
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
    /// Printer's marks, in drawing space (separated for the sheet's ink).
    pub marks: Option<Node>,
    /// Invert the page: paper black, ink clear (a film negative).
    pub negative: bool,
    /// The plate the sheet prints ([`PrintPlan::plates`]); `None`: composite.
    pub plate: Option<usize>,
    /// The ink of a separation, with its screen.
    pub ink: Option<PrintInk>,
}

impl PlannedSheet {
    fn sheet(&self) -> Sheet<'_> {
        Sheet {
            size: self.size,
            trim: self.trim,
            bleed: self.bleed,
            view: self.view,
            window: self.window,
            place: self.place,
            area: self.area,
            clip: true,
            marks: self.marks.as_ref(),
            negative: self.negative,
            background: None,
        }
    }
}

/// A laid-out job: the documents it draws, its sheets and the order they print in.
#[derive(Debug)]
pub struct PrintPlan<'a> {
    /// The document as it prints: live effects applied, the layers that print.
    pub doc: Cow<'a, Document>,
    /// With separations, one grey document per ink that prints (its coverage).
    pub plates: Vec<Document>,
    /// One copy's sheets.
    pub sheets: Vec<PlannedSheet>,
    /// Every page in print order, copies included: indices into `sheets`.
    pub order: Vec<usize>,
    pub title: String,
    /// When the job ran (Unix seconds, UTC).
    pub created: Option<i64>,
    pub warnings: Vec<String>,
}

/// Lay out printing `doc` as `opts` say, for a writer to draw.
pub fn plan<'a>(doc: &'a Document, opts: &PrintOptions) -> Result<PrintPlan<'a>, PdfError> {
    let set = &opts.settings;
    let job = Job::new(doc, set, opts.postscript)?;
    let title = opts.title.clone().unwrap_or_else(|| job.doc.title.clone());
    let created = opts.created.or_else(vectorcraft_doc::metadata::now_unix);
    let sep = Separator::new(&job.doc, set);
    let plates: Vec<Document> = (0..job.plates.len()).filter_map(|p| job.ink(Some(p))).map(|i| sep.plate(&i.name)).collect();
    let marks = set.marks.printer_marks();
    let sheets = job
        .sheets
        .iter()
        .filter_map(|(l, plate)| {
            let l = job.layouts.get(*l)?;
            let ink = job.ink(*plate);
            let art = marks_art(&job.doc, &marks, l, ink, &title, created).map(|mut art| {
                if let Some(ink) = ink {
                    sep.node(&ink.name, &mut art);
                }
                art
            });
            Some(PlannedSheet {
                size: l.size,
                trim: l.page_trim,
                bleed: l.page_bleed,
                view: l.view,
                window: l.window,
                place: l.place,
                area: l.area,
                marks: art,
                negative: set.output.image == PrintImage::Negative,
                plate: *plate,
                ink: ink.cloned(),
            })
        })
        .collect();
    let order = job.order(set);
    Ok(PrintPlan { plates, sheets, order, title, created, warnings: job.warnings, doc: job.doc })
}

/// Print `doc` to a PDF as `opts` say.
pub fn print(doc: &Document, opts: &PrintOptions) -> Result<PrintReport, PdfError> {
    let plan = plan(doc, opts)?;
    let doc = &*plan.doc;
    let separations = opts.settings.output.mode == OutputMode::Separations;
    // Plates are greys already: only composite colours go to the printer profile.
    let (output, advanced) = if separations { Default::default() } else { composite_pdf(&opts.settings) };
    let compression = CompressionSettings { compress_text: !opts.uncompressed, ..Default::default() };
    let pdf = PdfSettings { compression, output, advanced, ..Default::default() };
    let mut w = Writer::new(doc, &pdf, &plan.title, plan.created, !separations && doc.color_mode == ColorMode::Cmyk)?;
    // One exporter per ink (one for composite), each drawing its own document.
    let mut exporters: Vec<Exporter> =
        if separations { plan.plates.iter().map(|d| Exporter::new(d, &pdf)).collect() } else { vec![Exporter::new(doc, &pdf)] };
    for ex in &mut exporters {
        ex.non_printing = true;
        ex.intent = opts.settings.color.intent;
        // The writer's colour output: the printer profile's conversion.
        ex.out = w.out.clone();
        // Composite output keeps overprints as the Advanced option says (plates multiply them).
        ex.overprint = !separations && pdf.advanced.overprint == Overprint::Preserve;
    }
    for &i in &plan.order {
        let Some(s) = plan.sheets.get(i) else { continue };
        let Some(ex) = exporters.get_mut(s.plate.unwrap_or(0)) else { continue };
        w.page(ex, &s.sheet())?;
    }
    let overprinted = exporters.iter().any(|ex| ex.overprinted);
    for ex in exporters {
        w.absorb(ex);
    }
    let (bytes, more) = w.finish()?;
    let bytes = crate::forms::finish(bytes, doc, &pdf, false, overprinted)?;
    let mut warnings = plan.warnings;
    for m in more {
        if !warnings.contains(&m) {
            warnings.push(m);
        }
    }
    Ok(PrintReport { bytes, pages: plan.order.len(), warnings })
}

/// The printer's marks of page `l` (`None` without marks), with its page information: the
/// title, artboard and date, the tile, and the ink with its screen.
fn marks_art(doc: &Document, marks: &PrinterMarks, l: &Layout, ink: Option<&PrintInk>, title: &str, created: Option<i64>) -> Option<Node> {
    if !marks.any() {
        return None;
    }
    let mut info = String::new();
    if marks.page_info {
        info = crate::marks::page_info_of(doc, title, l.artboard, created);
        if let Some((t, n)) = l.tile {
            info.push_str(&format!("  ·  tile {} of {n}", t + 1));
        }
        if let Some(ink) = ink {
            info.push_str(&format!("  ·  {} {} lpi {}°", ink.name, ink.frequency, ink.angle));
        }
    }
    let boxes = PageBoxes::new(l.trim, l.bleed, marks);
    crate::marks::art(doc, marks, &boxes, l.bleed, &info)
}
