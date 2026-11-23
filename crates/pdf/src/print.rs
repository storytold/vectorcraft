//! File → Print as a print-ready PDF: each page is a sheet of the chosen paper with the art laid
//! out on it as the [`PrintSettings`] say.
//!
//! - General: copies (collated or not), reverse order, the artboards (all, a range or ignored:
//!   all the art as one page), blank artboards skipped, the paper and its orientation (or turned
//!   to each artboard's), transverse, which layers print (template layers never do), the
//!   placement on the imageable area (or where the Print Tiling tool put the pages,
//!   [`TileOrigin`]), and the scale: none, fit, custom, or tiles of the paper or of its imageable
//!   area, overlapping, a range of them. Without tiles, copies per sheet repeats the artboard
//!   (with its bleed and marks) across then down the imageable area, as many times as fit,
//!   `gutter` apart, and places the grid as it places one copy.
//! - Marks and bleed as PDF export draws them ([`crate::MarkSettings`], [`crate::BleedSettings`]),
//!   around the artboard on the paper, at the paper's scale.
//! - Output: composite, or separations, one page per ink that prints ([`print_inks`]) in the
//!   gray of its coverage (overprints honored, every spot color or chosen ones as process if
//!   asked), emulsion down
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
use std::collections::HashSet;
use std::sync::Arc;

use kurbo::{Affine, Rect, Vec2};
use serde::Serialize;
use vectorcraft_color::cms::{self, ProfileKind};
use vectorcraft_doc::marks::PrinterMarks;
use vectorcraft_doc::overprint::{OverprintBlack, clear_overprints, darken_overprints};
use vectorcraft_doc::{ColorMode, Document, Node, NodeKind};

pub use layout::{MAX_COPIES_PER_SHEET, MAX_TILES, TileGrid};
pub use plates::{document_inks, print_inks};
pub use settings::*;
pub use tiling::{TilingPage, tiling};

use crate::export::{Exporter, Sheet, Writer, leaving_out_undecodable};
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
    /// The job goes to a print queue, which puts each page on the paper as it is (macOS does
    /// not turn a landscape page to the paper): a page whose paper the layout turned to landscape
    /// is written upright, the art turned a quarter anticlockwise, as a printer driver turns a
    /// landscape job. Paper that is wide in Portrait, and transverse pages, stay as they are.
    pub for_printer: bool,
}

/// A printed job.
#[derive(Clone, Debug)]
pub struct PrintReport {
    pub bytes: Vec<u8>,
    pub pages: usize,
    /// What the job leaves out of the document or prints differently.
    pub warnings: Vec<String>,
    /// Notes on the options: one the job didn't apply or applied another way.
    pub setting_notes: Vec<String>,
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
    /// The artboard (trim box) on the page, `[x0, y0, x1, y1]`; with copies per sheet, the box
    /// around every copy's.
    pub trim: [f64; 4],
    /// With copies per sheet, each copy's artboard on the page, `[x0, y0, x1, y1]`, across then
    /// down (the first is where `transform` puts it).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<[f64; 4]>,
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
    /// Every ink of the document as a separation lists it, the spot inks printing as process
    /// (which `inks` leaves out) included ([`document_inks`]); empty for composite output.
    pub document_inks: Vec<PrintInk>,
    /// What the job leaves out of the document or prints differently.
    pub warnings: Vec<String>,
    /// Notes on the options: one the job doesn't apply or applies another way. They aren't
    /// serialized: a reply lists them in `warnings`, after what the job leaves out.
    #[serde(skip)]
    pub setting_notes: Vec<String>,
}

/// A laid-out job.
struct Job<'a> {
    /// The document as it prints: live effects applied, the layers that print.
    doc: Cow<'a, Document>,
    layouts: Vec<Layout>,
    tiles: Vec<TileGrid>,
    /// The inks of a separation (every one, printed or not).
    inks: Vec<PrintInk>,
    /// Every ink of the document, the spot inks printing as process included.
    document_inks: Vec<PrintInk>,
    /// The inks that print (indices into `inks`): each a plate.
    plates: Vec<usize>,
    /// One copy's pages: (layout, plate).
    sheets: Vec<(usize, Option<usize>)>,
    /// What the job leaves out of the document or prints differently.
    warnings: Vec<String>,
    /// Notes on the options: one the job doesn't apply or applies another way.
    setting_notes: Vec<String>,
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
/// says, and black art overprinting in separations with [`PrintOutput::overprint_black`].
pub fn printed_document<'a>(doc: &'a Document, set: &PrintSettings) -> Cow<'a, Document> {
    // The layers that do not print are left out first, so the art placed for symbols is only
    // the art that prints; the layers kept are shown.
    let mut kept = doc.clone();
    keep_layers(&mut kept.layers, set.print_layers);
    let layers = vectorcraft_effects::WrittenLayers { hidden: false, non_printing: true, hidden_objects: false };
    let mut d = vectorcraft_effects::bake_document_placing_symbols(&kept, true, layers).unwrap_or(kept);
    composite_overprints(&mut d, set);
    separation_overprint_black(&mut d, set);
    Cow::Owned(d)
}

/// Output › Overprint Black in separations: every fill and stroke of solid 100% process black or
/// 100% gray, each to within half a percent ([`vectorcraft_doc::overprint::is_black_ink`]), or a
/// gradient whose every stop is (every point of a freeform one), the characters' too, overprints. Rich black, RGB and Lab black,
/// tints below 99.5%, and colors linked to a spot swatch or to Registration keep
/// knocking out ([`OverprintBlack::default`]). Art that overprints already keeps doing so. A
/// symbol instance that repaints its art is judged by the paints it prints with
/// ([`place_repainted_instances`]).
fn separation_overprint_black(doc: &mut Document, set: &PrintSettings) {
    if set.output.mode != OutputMode::Separations || !set.output.overprint_black {
        return;
    }
    place_repainted_instances(doc);
    let mut spots = doc.spot_names();
    spots.push(vectorcraft_color::swatch::REGISTRATION.to_string());
    let op = OverprintBlack::default();
    let symbols = doc.symbols.iter_mut().map(|s| &mut s.art);
    for n in doc.layers.iter_mut().chain(symbols).chain(doc.patterns.iter_mut().flat_map(|p| p.art.iter_mut())) {
        op.apply_deep(n, &spots);
    }
}

/// Replace each symbol instance in `doc`'s layers and patterns whose own fill or stroke changes
/// ([`vectorcraft_doc::symbol_overrides::SymbolOverride`]) or fill stain repaint its symbol's art
/// ([`repaints`]), or whose symbol's art holds such an instance at any depth, with that art as
/// the instance prints it ([`Placing::instance`]). Then the same is done inside the art of the
/// symbols type shows as inline graphics ([`inline_symbols`]), which draw their symbol's art as it
/// is; the other symbols' own art stays as it is, and so do hidden objects and template layers.
/// Once the layers and patterns, with the art placed for symbols already, hold
/// [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects, the remaining instances stay as they are, and so
/// do instances whose art would draw deeper than [`vectorcraft_doc::depth::MAX_DRAWN_DEPTH`].
fn place_repainted_instances(doc: &mut Document) {
    let holders = repainting_symbols(doc);
    let art = || doc.layers.iter().chain(doc.patterns.iter().flat_map(|p| p.art.iter()));
    if holders.is_empty() && !art().any(|n| holds_repaint(n, &holders)) {
        return;
    }
    let held = art().map(|n| n.count_with_masks() as u64).fold(0u64, u64::saturating_add);
    let mut placing =
        Placing { holders, heights: vectorcraft_doc::depth::DrawnHeights::of(doc), budget: vectorcraft_doc::MAX_SYMBOL_NODES.saturating_sub(held) };
    // The art placed for an instance holds the instances inside it: each pass places those, up
    // to the depth objects are drawn to.
    for _ in 0..=vectorcraft_doc::depth::MAX_DRAWN_DEPTH {
        let mut changed = false;
        for i in 0..doc.layers.len() {
            if let Some(layer) = placing.subtree(doc, &doc.layers[i], 1) {
                doc.layers[i] = layer;
                changed = true;
            }
        }
        for p in 0..doc.patterns.len() {
            for i in 0..doc.patterns[p].art.len() {
                if let Some(art) = placing.subtree(doc, &doc.patterns[p].art[i], 1) {
                    doc.patterns[p].art[i] = art;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let inline = inline_symbols(doc);
    for _ in 0..=vectorcraft_doc::depth::MAX_DRAWN_DEPTH {
        let mut changed = false;
        for i in 0..doc.symbols.len() {
            if inline.contains(&doc.symbols[i].name)
                && let Some(art) = placing.subtree(doc, &doc.symbols[i].art, 1)
            {
                doc.symbols[i].art = art;
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

/// The names of the symbols that type in `doc` shows as inline graphics, in its layers, symbols
/// and patterns.
fn inline_symbols(doc: &Document) -> HashSet<String> {
    fn visit(n: &Node, out: &mut HashSet<String>) {
        if let NodeKind::Text(t) = &n.kind {
            out.extend(t.runs.iter().filter_map(|r| r.inline.as_ref()).map(|i| i.symbol.clone()));
        }
        for c in n.children().into_iter().flatten() {
            visit(c, out);
        }
    }
    let mut out = HashSet::new();
    for n in doc.layers.iter().chain(doc.symbols.iter().map(|s| &s.art)).chain(doc.patterns.iter().flat_map(|p| p.art.iter())) {
        visit(n, &mut out);
    }
    out
}

/// Does symbol instance `n` repaint its symbol's art: with its own fill or stroke changes, or with
/// its fill's stain?
fn repaints(n: &Node) -> bool {
    matches!(n.kind, NodeKind::SymbolInstance { .. })
        && (n.extras().symbol_overrides.iter().any(|o| o.fill.is_some() || o.stroke.is_some())
            || vectorcraft_doc::symbol_overrides::stain(n).is_some())
}

/// Does `n`'s subtree hold a repainting instance ([`repaints`]) or an instance of one of the
/// symbols `holders`?
fn holds_repaint(n: &Node, holders: &HashSet<String>) -> bool {
    repaints(n)
        || matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if holders.contains(symbol))
        || n.children().is_some_and(|c| c.iter().any(|c| holds_repaint(c, holders)))
}

/// The names of the symbols whose art holds a repainting instance ([`repaints`]) at any depth,
/// through the instances of other symbols too.
fn repainting_symbols(doc: &Document) -> HashSet<String> {
    let mut out = HashSet::new();
    loop {
        let before = out.len();
        for s in &doc.symbols {
            if !out.contains(&s.name) && holds_repaint(&s.art, &out) {
                out.insert(s.name.clone());
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// What [`place_repainted_instances`] places instances with.
struct Placing {
    /// The symbols whose art holds a repainting instance ([`repainting_symbols`]).
    holders: HashSet<String>,
    heights: vectorcraft_doc::depth::DrawnHeights,
    /// The objects left to place.
    budget: u64,
}

impl Placing {
    /// `n`'s subtree, `n` placed `level` deep, with its instances that [`Self::instance`] places
    /// placed, copying only the nodes on the way to them; `None` when there is none. Hidden
    /// objects and template layers do not print and are left as they are.
    fn subtree(&mut self, doc: &Document, n: &Arc<Node>, level: usize) -> Option<Arc<Node>> {
        if !n.visible || matches!(n.kind, NodeKind::Layer { template: true, .. }) {
            return None;
        }
        if let Some(g) = self.instance(doc, n, level) {
            return Some(Arc::new(g));
        }
        let members: Vec<(usize, Arc<Node>)> =
            n.children().into_iter().flatten().enumerate().filter_map(|(i, c)| Some((i, self.subtree(doc, c, level + 1)?))).collect();
        if members.is_empty() {
            return None;
        }
        let mut copy = Arc::clone(n);
        if let Some(children) = Arc::make_mut(&mut copy).children_mut() {
            for (i, c) in members {
                if let Some(slot) = children.get_mut(i) {
                    *slot = c;
                }
            }
        }
        Some(copy)
    }

    /// Symbol instance `n`, placed `level` deep, when it repaints its art ([`repaints`]) or its
    /// symbol is one of [`Self::holders`], as a group of that art placed as the exporter places
    /// it, with the instance's id, name, visibility, transparency, opacity mask, effects and
    /// attributes. `None` for other objects, for instances of a missing symbol, when the art does
    /// not fit in [`Self::budget`] (which it is taken from), and when it would draw deeper than
    /// [`vectorcraft_doc::depth::MAX_DRAWN_DEPTH`].
    fn instance(&mut self, doc: &Document, n: &Node, level: usize) -> Option<Node> {
        let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return None };
        if !repaints(n) && !self.holders.contains(symbol) {
            return None;
        }
        let sym = doc.symbols.iter().find(|s| s.name == *symbol)?;
        let art = vectorcraft_doc::symbol_overrides::instance_art(&sym.art, n);
        // The most objects placing it can make, counted before it is placed.
        let most = doc.symbol_slice_map(symbol, *xf).map_or(art.count_with_masks() as u64, |m| m.most_placed(&art, doc));
        let left = self.budget.checked_sub(most)?;
        let placed = doc.place_symbol_art(symbol, &art, *xf)?;
        let mut g = Node::group(n.id, vec![Arc::new(placed)]);
        g.name = n.name.clone();
        g.visible = n.visible;
        g.opacity = n.opacity;
        g.blend = n.blend;
        g.isolate = n.isolate;
        // One element of an enclosing knockout group, as the instance is: its pieces never knock
        // each other out.
        g.knockout = vectorcraft_doc::Knockout::Off;
        g.knockout_shape = n.knockout_shape;
        g.appearance.effects = n.appearance.effects.clone();
        g.extras_mut().mask = n.extras().mask.clone();
        g.extras_mut().attrs = n.extras().attrs.clone();
        if self.heights.deepest(&g, level) > vectorcraft_doc::depth::MAX_DRAWN_DEPTH {
            return None;
        }
        self.budget = left;
        Some(g)
    }
}

/// Overprinting fills and strokes in composite output: discarded (they knock out) or simulated
/// (drawn with Darken, as Overprint Preview shows them), after which nothing overprints any
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
            darken_overprints(n, discard_white, &doc.images);
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
        let (mut warnings, mut setting_notes) = (vec![], vec![]);
        let (layouts, tiles) = layout::layout(&doc, set, &mut warnings, &mut setting_notes)?;
        let separations = set.output.mode == OutputMode::Separations;
        let (document_inks, more) = if separations { document_inks(&doc, set) } else { Default::default() };
        setting_notes.extend(more);
        let inks: Vec<PrintInk> = document_inks.iter().filter(|i| !i.convert_to_process).cloned().collect();
        let plates: Vec<usize> = (0..inks.len()).filter(|i| inks[*i].print).collect();
        let printed: Vec<Option<usize>> = if separations { (0..plates.len()).map(Some).collect() } else { vec![None] };
        if printed.is_empty() {
            return Err(PdfError::BadSetting("output.inks: no ink is set to print".into()));
        }
        // Counted before the sheets are made: layouts times inks can be far more than the limit.
        if layouts.len().saturating_mul(printed.len()).saturating_mul(set.copies as usize) > MAX_PAGES {
            return Err(PdfError::BadSetting(format!("the job would print more than {MAX_PAGES} pages")));
        }
        let sheets: Vec<(usize, Option<usize>)> = (0..layouts.len()).flat_map(|l| printed.iter().map(move |i| (l, *i))).collect();
        if sheets.len().saturating_mul(set.copies as usize) > MAX_PAGES {
            return Err(PdfError::BadSetting(format!("the job would print more than {MAX_PAGES} pages")));
        }
        if separations && doc.color_mode == ColorMode::Rgb {
            warnings.push("the document is RGB: separations convert its colours to CMYK with the colour settings".into());
        }
        if !postscript && separations && set.output.inks.iter().any(|i| i.frequency.is_some() || i.angle.is_some()) {
            setting_notes.push("ink frequencies and angles are not written as halftone screens yet: the output device's screens apply".into());
        }
        if separations && set.advanced.print_as_bitmap {
            setting_notes.push("Print as Bitmap is for composite output: separations print the art as it is".into());
        }
        if separations && cms::profile(set.color.profile.trim()).is_some_and(|p| p.kind != ProfileKind::Cmyk) {
            setting_notes.push("separations separate with a CMYK profile: the RGB printer profile is left out".into());
        }
        if !postscript && !set.graphics.auto_flatness {
            setting_notes.push("a fixed flatness is not written yet: the output device flattens curves".into());
        }
        Ok(Self { doc, layouts, tiles, inks, document_inks, plates, sheets, warnings, setting_notes })
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
                copies: l.copies.iter().map(|m| l.view.transform_rect_bbox(l.trim + *m)).map(|r| [r.x0, r.y0, r.x1, r.y1]).collect(),
            })
        })
        .collect();
    Ok(PrintPreview {
        pages: job.sheets.len() * set.copies.max(1) as usize,
        sheets,
        tiles: job.tiles,
        inks: job.inks,
        document_inks: job.document_inks,
        warnings: job.warnings,
        setting_notes: job.setting_notes,
    })
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
    /// With copies per sheet, each copy's move in drawing space from where [`Self::place`] puts
    /// the art, across then down (the first is zero): the art and the marks are drawn once per
    /// copy. Empty: one copy.
    pub copies: Vec<Vec2>,
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
            unit: 1.0,
            trim: self.trim,
            bleed: self.bleed,
            view: self.view,
            window: self.window,
            place: self.place,
            copies: &self.copies,
            area: self.area,
            clip: true,
            marks: self.marks.as_ref(),
            negative: self.negative,
            // A printed sheet has no links to click.
            links: false,
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
    /// What the job leaves out of the document or prints differently.
    pub warnings: Vec<String>,
    /// Notes on the options: one the job doesn't apply or applies another way.
    pub setting_notes: Vec<String>,
}

/// Lay out printing `doc` as `opts` say, for a writer to draw.
pub fn plan<'a>(doc: &'a Document, opts: &PrintOptions) -> Result<PrintPlan<'a>, PdfError> {
    let set = &opts.settings;
    let job = Job::new(doc, set, opts.postscript)?;
    let title = opts.title.clone().unwrap_or_else(|| job.doc.title.clone());
    let created = opts.created.or_else(vectorcraft_doc::metadata::output_time);
    let sep = Separator::new(&job.doc, set, opts.postscript);
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
            let sheet = PlannedSheet {
                size: l.size,
                trim: l.page_trim,
                bleed: l.page_bleed,
                view: l.view,
                window: l.window,
                place: l.place,
                copies: l.copies.clone(),
                area: l.area,
                marks: art,
                negative: set.output.image == PrintImage::Negative,
                plate: *plate,
                ink: ink.cloned(),
            };
            // Only the paper the layout turned: custom paper can be wide in Portrait.
            let turned = l.orientation.landscape() && !set.transverse;
            Some(if opts.for_printer && turned { upright(sheet) } else { sheet })
        })
        .collect();
    let order = job.order(set);
    Ok(PrintPlan { plates, sheets, order, title, created, warnings: job.warnings, setting_notes: job.setting_notes, doc: job.doc })
}

/// Landscape sheet `s` upright on the paper ([`PrintOptions::for_printer`]): a page wider than
/// tall turns a quarter anticlockwise, its top to the paper's left edge.
fn upright(mut s: PlannedSheet) -> PlannedSheet {
    let (w, h) = s.size;
    if w <= h {
        return s;
    }
    // y down: the top-right corner (w, 0) goes to the top-left, the top-left to the bottom-left.
    let turn = Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, w]);
    s.size = (h, w);
    s.view = turn * s.view;
    s.trim = turn.transform_rect_bbox(s.trim);
    s.bleed = turn.transform_rect_bbox(s.bleed);
    s
}

/// Print `doc` to a PDF as `opts` say.
pub fn print(doc: &Document, opts: &PrintOptions) -> Result<PrintReport, PdfError> {
    craft_pdf::write::guard(|| leaving_out_undecodable(doc, |left_out| print_inner(doc, opts, left_out)))
}

fn print_inner(doc: &Document, opts: &PrintOptions, left_out: &mut HashSet<String>) -> Result<PrintReport, PdfError> {
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
        // Composite output keeps overprints as the Advanced option says (plates darken them).
        ex.overprint = !separations && pdf.advanced.overprint == Overprint::Preserve;
        // A plate's overprinting characters darken, as its fills and strokes do.
        ex.darken_overprints = separations;
        ex.left_out = left_out.clone();
    }
    for &i in &plan.order {
        let Some(s) = plan.sheets.get(i) else { continue };
        let Some(ex) = exporters.get_mut(s.plate.unwrap_or(0)) else { continue };
        w.page(ex, &s.sheet())?;
    }
    let overprinted = exporters.iter().any(|ex| ex.overprinted);
    let invisible = exporters.iter().any(|ex| ex.invisible);
    for ex in exporters {
        w.absorb(ex);
    }
    let (bytes, more, settings) = w.finish(left_out, false)?;
    let bytes = crate::forms::finish(bytes, doc, &pdf, false, overprinted, invisible)?;
    let mut warnings = plan.warnings;
    for m in more {
        if !warnings.contains(&m) {
            warnings.push(m);
        }
    }
    let mut setting_notes: Vec<String> = vec![];
    for m in plan.setting_notes.into_iter().chain(settings) {
        if !warnings.contains(&m) && !setting_notes.contains(&m) {
            setting_notes.push(m);
        }
    }
    Ok(PrintReport { bytes, pages: plan.order.len(), warnings, setting_notes })
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
