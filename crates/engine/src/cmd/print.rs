//! File → Print as a print-ready PDF or a PostScript file (`file.print`), what it would print
//! (`print.preview`) and the print settings saved with the document (`print.setup`,
//! [`vectorcraft_doc::Document::print_setup`]).

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_eps::{Level, PrintFonts, PrintJob, PrintPage};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_pdf::{PrintLayers, PrintOptions, PrintPreview, PrintSettings};

use super::fileio::pdf::{merge, pdf_error};
use super::fileio::{extension, write_or_return};
use super::flatten::has_transparent_tile_images;
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, cmd, has_doc, str_param, u64_req};

const SETUP: &str = "print.setup";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "print.setup",
            "Print Setup",
            [],
            None,
            "{preset?: name ([Default] or a saved print preset, any case: print.presets.list; its settings replace the document's, as choosing it in the Print dialog does), settings?: {copies: 1 (1–999), collate: true, reverse, artboards: all|range|ignore (all the art as one page), range: \"1-3, 5\" (1-based, with artboards: range), skipBlank (leave out artboards with no art that prints), media: letter|legal|tabloid|a3|a4|a5|a6|b4|b5|b4 (jis)|b5 (jis)|b6 (jis)|japanese postcard|japanese double postcard|japanese envelope #3|japanese envelope #4|kaku #2|kaku #3|yonroku-ban|custom (a paper is named by its id or its dialog label, any case and spacing, so `B5 (JIS)` works; b4 and b5 stay the ISO 216 sizes, 250 × 353 and 176 × 250 mm, and the JIS sizes are b4 (jis) 257 × 364 mm, b5 (jis) 182 × 257 mm and b6 (jis) 128 × 182 mm), width, height (custom paper, 72–14400 pt), orientation: portrait|landscape|portraitFlipped|landscapeFlipped, autoRotate: true (turn the paper to each artboard; orientation ignored), transverse (the page a quarter turn on the paper), printLayers: visiblePrintable|visible|all (template layers never print), placement: {origin: topLeft|top|topRight|left|center|right|bottomLeft|bottom|bottomRight (the point of the printed area, the artboard with its bleed and marks, on the same point of the imageable area), x, y (pt, right and down)}, scaling: none|fit|custom|tileFull|tileImageable, scale: {width: 100, height: 100 (1–1000 % with custom and tiling), linked: true (width and height stay equal: give one and the other follows, and turning it on sets the height to the width; a different width and height without linked turn it off, and with linked: true, or linked: null over a linked scale, are an error; a saved scale without linked reads it as false)}, overlap: 0 (pt between tiles), tileRange: \"\" (1-based tiles across then down; empty: all), copiesPerSheet: false (not with tiling: as many copies of each artboard, with its bleed and marks, as fit the imageable area, across then down, not turned; the placement places the grid; an artboard that doesn't fit prints one copy per sheet, with a warning), gutter: 0 (pt between copies, 0–720), margin: 0 (pt the device can't print around the paper: the imageable area is inside it), marks: {trim, registration, colorBars, pageInfo, kind: roman|japanese, weight: 0.25, offset: 6} (as document.exportPdf, at the paper's scale; page information adds the tile and the ink), bleed: {useDocument: true (the document's bleed, document.setup), top, bottom, left, right} (pt), output: {mode: composite|separations (one gray page per ink that prints), emulsion: up|down (down mirrors), image: positive|negative, spotsToProcess, overprintBlack: false (separations: solid 100% K or 100% gray fills, strokes and characters, to within half a percent, overprint in the printed copy, gradients when every stop is (every point of a freeform one); rich, RGB, spot-linked and lighter blacks are not marked, and paints already set to overprint keep it; composite ignores it), inks: [{name: \"Cyan\"|…|a spot swatch, print: true, frequency (lpi, default 60), angle (screen angle, degrees counter-clockwise as PostScript setscreen takes it; default C 15, M 75, Y 0, K 45, spots 45), convertToProcess: false (a spot ink only: it prints on the process plates, with no plate of its own, and its print, frequency and angle are kept for when it prints as a spot ink again; spotsToProcess converts every spot ink whatever it says)}]}, graphics: {autoFlatness: true, flatness: 1 (0.2–100), fonts: none|subset|complete (PostScript output: subset (default) sends each page the glyphs it shows, complete sends every glyph of each font once before the first page, none sends no font and names the printer's own; PDF output embeds subsets of the fonts whose licenses allow it, and draws the others as outlines, with any of them, and none or complete adds a note)}, color: {intent: perceptual|relativeColorimetric|saturation|absoluteColorimetric, preserveNumbers: true (CMYK colors keep their values), profile: \"\" (the printer profile, an RGB or CMYK one: composite PDF colors are converted to it with the intent, separations separate with a CMYK one; empty: as they are)}, advanced: {printAsBitmap (composite pages print as one image each, at the document's raster effects resolution, or lower for a page larger than 64 Mpx or 32,768 px a side), overprints: preserve|discard|simulate (composite output; preserve writes them to overprint (/OP), simulate prints them as Overprint Preview shows them; separations always keep them), flattenerPreset: \"\" (a flattener preset, built-in or saved: transparency is flattened before printing; empty: it prints live; PostScript defaults to medium)}, tileOrigin: {placed: false, x, y} (where the Print Tiling tool put the pages: print.tiling.set)}} store the print settings with the document, over the ones it has or the preset's (null keeps a value), as one undo step; no settings or preset → {settings} (the current ones, defaults if never set up)",
            has_doc,
            setup
        ),
        cmd!(
            query "print.preview",
            "Print Preview",
            [],
            None,
            "{preset?: name (as print.setup's: its settings instead of the document's), settings?: {…settings of print.setup} (over the document's or the preset's)} → {pages (copies included), sheets: [{artboard (0-based; null with artboards ignored), tile? (1-based), ink? (separations), width, height (pt, the page), orientation, scale: [width %, height %], transform: [a, b, c, d, e, f] (document space → the page, pt, y down), area: [x0, y0, x1, y1] (the document region the page prints), trim: [x0, y0, x1, y1] (the artboard on the page; with copies per sheet, the box around every copy's), copies?: [[x0, y0, x1, y1]…] (copies per sheet: each copy's artboard on the page, across then down)}] (one copy, in order), tiles: [{artboard, columns, rows, printed: [1-based tiles], tiles: [[x0, y0, x1, y1]…] (document space)}], inks: [{name, spot, print, frequency, angle}] (separations: the plates), documentInks: [{name, spot, print, frequency, angle, convertToProcess?: true (a spot ink printing as process, with no plate)}] (separations: every ink of the document), warnings (art larger than the imageable area, type that prints in the fallback font, type that doesn't fit its frame, options not applied yet…), settings} without printing",
            has_doc,
            preview
        ),
        cmd!(
            "file.print",
            "Print",
            [],
            Some("Cmd+P"),
            "{preset?: name (as print.setup's: its settings instead of the document's; the saved settings stay), settings?: {…settings of print.setup} (over the document's or the preset's), includeArtboardFill?: true (each artboard's background color prints behind its art; false leaves it out), path?, format?: pdf (default: a print-ready PDF) | postscript (the default for a .ps path: a PostScript file with DSC comments, %%Pages, a page per sheet with its paper size, marks, separations with their ink and halftone screen, the fixed flatness, negatives), level?: 3|2 (the PostScript language level), flattenerPreset?: \"medium\" (PostScript: the preset transparency is flattened with — high, low or a saved one, see flattener.presets.list), acknowledgeLoss?: bool (with path: write over the file the document was read from although reading it left things out), maxPages?: n (a whole number: the job is composed only when it prints at most n pages, copies counted; with more, nothing is composed or written and the reply is {pages, format}), forPrinter?: bool (the PDF goes to a print queue: a page whose paper is turned to landscape is written upright, the art turned a quarter anticlockwise, as a printer driver turns a landscape job; macOS prints a landscape page as it is, cut off on upright paper; paper wide in Portrait and transverse pages stay as they are)} print the document, one page per sheet of paper (per tile, per ink, per copy; with copiesPerSheet, a sheet holds as many copies of an artboard as fit) → {pages, format, path, bytes, warnings}; no path → {dataBase64, bytes, pages, format, warnings}. Raster effects print at the document's raster effects resolution; a PDF leaves halftone screens and flatness to the output device",
            has_doc,
            print
        ),
    ]
}

/// The print settings saved with `doc` (defaults if never set up) as JSON.
fn saved(doc: &Document) -> Value {
    doc.print_setup.clone().unwrap_or_else(|| serde_json::to_value(PrintSettings::default()).unwrap_or_default())
}

/// The settings of `doc` with `p.settings` over them, checked.
pub(crate) fn settings(cmd: &str, doc: &Document, p: &Value) -> Result<PrintSettings> {
    settings_over(cmd, saved(doc), p)
}

/// The settings `p` asks for: those of the print preset `p.preset` names ([Default] or a saved
/// one, any case), else those saved with `doc`, with `p.settings` over them, checked.
fn preset_or_saved(cmd: &str, s: &Session, doc: &Document, p: &Value) -> Result<PrintSettings> {
    let base = match p.get("preset") {
        None | Some(Value::Null) => saved(doc),
        Some(Value::String(name)) => {
            let set = s.print_preset_settings(name).ok_or_else(|| {
                let names: Vec<String> = s.print_presets().into_iter().map(|q| q.name).collect();
                bad(cmd, format!("no print preset named `{name}` (presets: {})", names.join(", ")))
            })?;
            to_json(&set)?
        }
        Some(_) => return Err(bad(cmd, "preset must be the name of a print preset")),
    };
    settings_over(cmd, base, p)
}

/// The settings `v` (JSON) with `p.settings` over them, checked.
pub(crate) fn settings_over(cmd: &str, mut v: Value, p: &Value) -> Result<PrintSettings> {
    match p.get("settings") {
        None | Some(Value::Null) => {}
        Some(over @ Value::Object(_)) => merge(&mut v, over),
        Some(_) => return Err(bad(cmd, "settings must be an object")),
    }
    link_scale(cmd, &mut v, p.get("settings").and_then(|s| s.get("scale")).filter(|s| s.is_object()))?;
    let set = PrintSettings::deserialize(&v).map_err(|e| bad(cmd, format!("print settings: {e}")))?;
    set.check().map_err(|e| pdf_error(cmd, e))?;
    Ok(set)
}

/// Keep the custom scale's width and height equal in the merged settings `v` while `scale.linked`
/// is on, after `over` (the request's `scale`) was merged in: the one side the request gives sets
/// the other, and otherwise the height takes the width (as turning the link on does). A request
/// that gives a different width and height without a `linked` key turns the link off; with
/// `linked: true`, or with `linked: null` over a linked scale, it is refused.
fn link_scale(cmd: &str, v: &mut Value, over: Option<&Value>) -> Result<()> {
    let given = |k: &str| over.and_then(|o| o.get(k)).filter(|x| !x.is_null());
    let (width, height) = (given("width").cloned(), given("height").cloned());
    let apart = matches!((&width, &height), (Some(w), Some(h)) if w.as_f64() != h.as_f64());
    let Some(scale) = v.get_mut("scale").and_then(Value::as_object_mut) else { return Ok(()) };
    if apart && over.and_then(|o| o.get("linked")).is_none() {
        scale.insert("linked".into(), Value::Bool(false));
    }
    if scale.get("linked") != Some(&Value::Bool(true)) {
        return Ok(());
    }
    if apart {
        return Err(bad(cmd, "scale: give one percentage, or the same width and height, while scale.linked is on (linked: false scales them apart)"));
    }
    let (from, to) = if width.is_none() && height.is_some() { ("height", "width") } else { ("width", "height") };
    if let Some(value) = scale.get(from).cloned() {
        scale.insert(to.into(), value);
    }
    Ok(())
}

pub(crate) fn to_json(set: &PrintSettings) -> Result<Value> {
    serde_json::to_value(set).map_err(|e| EngineError::Other(e.to_string()))
}

fn setup(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = &s.doc()?.doc;
    let set = to_json(&preset_or_saved(SETUP, s, doc, p)?)?;
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if (given("settings") || given("preset")) && doc.print_setup.as_ref() != Some(&set) {
        let stored = set.clone();
        s.edit("Print Setup", |d, _| {
            d.print_setup = Some(stored);
            Ok(())
        })?;
    }
    Ok(json!({ "settings": set }))
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "print.preview";
    let doc = &s.doc()?.doc;
    let set = preset_or_saved(C, s, doc, p)?;
    super::printadvanced::flattener(C, &set, &s.prefs.flattener_presets)?;
    let mut pv = vectorcraft_pdf::preview(doc, &set).map_err(|e| pdf_error(C, e))?;
    pv.warnings.extend(type_warnings(doc, set.print_layers, &areas(&pv)));
    // What the job loses comes first, then the notes on the options.
    let settings = std::mem::take(&mut pv.setting_notes);
    pv.warnings.extend(settings);
    let mut v = serde_json::to_value(pv).map_err(|e| EngineError::Other(e.to_string()))?;
    v["settings"] = to_json(&set)?;
    Ok(v)
}

/// Reuse the Print dialog's saved settings and page geometry for production preflight, without
/// printing, writing a file or duplicating its type warnings.
pub(super) struct PreflightContext {
    pub layers: PrintLayers,
    pub pages: Vec<(Rect, Affine)>,
    pub warnings: Vec<String>,
}

pub(super) fn preflight_context(s: &Session, indices: &[usize]) -> Result<PreflightContext> {
    let settings =
        json!({ "settings": { "artboards": "range", "range": indices.iter().map(|i| (i + 1).to_string()).collect::<Vec<_>>().join(",") } });
    let set = preset_or_saved("document.preflight", s, &s.doc()?.doc, &settings)?;
    super::printadvanced::flattener("document.preflight", &set, &s.prefs.flattener_presets)?;
    let mut pv = vectorcraft_pdf::preview(&s.doc()?.doc, &set).map_err(|e| pdf_error("document.preflight", e))?;
    let pages = pv
        .sheets
        .iter()
        .filter_map(|p| {
            let to_page = Affine::new(p.transform);
            on_page(Rect::new(p.area[0], p.area[1], p.area[2], p.area[3]), to_page, (p.width, p.height)).map(|area| (area, to_page))
        })
        .collect();
    pv.warnings.extend(pv.setting_notes);
    Ok(PreflightContext { layers: set.print_layers, pages, warnings: pv.warnings })
}

/// The parts of the document (document space) that the pages of `pv` print.
fn areas(pv: &PrintPreview) -> Vec<Rect> {
    pv.sheets
        .iter()
        .filter_map(|p| on_page(Rect::new(p.area[0], p.area[1], p.area[2], p.area[3]), Affine::new(p.transform), (p.width, p.height)))
        .collect()
}

/// The part of `area` (document space) that lands on a `w` × `h` page placed by `to_page`
/// (document space → the page); `None` when none of it does.
fn on_page(area: Rect, to_page: Affine, (w, h): (f64, f64)) -> Option<Rect> {
    let det = to_page.determinant();
    if !(det.is_finite() && det.abs() > 1e-12) {
        return Some(area);
    }
    let page = to_page.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
    let r = area.intersect(page);
    (r.area() > 0.0).then_some(r)
}

/// The warnings about the type in `doc` that prints (on the layers `which` prints, on pages that
/// print `areas`): type in the fallback font, and type that doesn't fit its frame.
fn type_warnings(doc: &Document, which: PrintLayers, areas: &[Rect]) -> Vec<String> {
    // Placed documents print their files' art, which names the resources this adds.
    let doc = doc.with_placed_art();
    let mut layers = doc.layers.clone();
    vectorcraft_pdf::keep_layers(&mut layers, which);
    let (fonts, overflow) = super::fonts::printed_type(&doc, &layers, areas);
    let overflow: Vec<_> = overflow.into_iter().collect();
    super::fonts::substitution_warning_of(fonts).into_iter().chain(super::typecmd::overflow_warning(&overflow)).collect()
}

const PRINT: &str = "file.print";

/// Whether `file.print` writes PostScript: `format`, else a `.ps` path.
fn to_postscript(p: &Value) -> Result<bool> {
    match str_param(p, "format") {
        None => Ok(str_param(p, "path").is_some_and(|path| extension(path) == "ps")),
        Some("postscript") => Ok(true),
        Some("pdf") => Ok(false),
        Some(f) => Err(bad(PRINT, format!("format `{f}`: pdf or postscript"))),
    }
}

fn print(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(path) = str_param(p, "path") {
        super::fileio::check_not_lossy_overwrite(s.doc()?, path, p, PRINT)?;
    }
    let postscript = to_postscript(p)?;
    let max_pages = match p.get("maxPages") {
        None | Some(Value::Null) => None,
        Some(_) => Some(u64_req(p, "maxPages", PRINT)?),
    };
    let too_many = |pages: usize| max_pages.is_some_and(|m| pages as u64 > m);
    let doc = &s.doc()?.doc;
    let mut set = preset_or_saved(PRINT, s, doc, p)?;
    if postscript {
        // PostScript has no transparency: `flattenerPreset`, else the Advanced preset, else medium.
        let given = str_param(p, "flattenerPreset").filter(|n| !n.trim().is_empty());
        let preset = given.or(Some(set.advanced.flattener_preset.as_str()).filter(|n| !n.trim().is_empty())).unwrap_or("medium");
        set.advanced.flattener_preset = preset.to_string();
    }
    // Placed documents print their files' art.
    let (full, _) = super::place::document::full_documents(doc);
    // The artboards' background colors print behind the art.
    let filled = super::fileio::with_artboard_fills(&full, p, 0..full.artboards.len()).map_err(|e| bad(PRINT, e))?;
    let doc = &*filled;
    // Raster effects print as images at the document's raster effects resolution.
    let flat = super::rasterfx::flatten_raster_effects(doc);
    let doc = flat.as_ref().unwrap_or(doc);
    let bitmap = set.advanced.print_as_bitmap && set.output.mode == vectorcraft_pdf::OutputMode::Composite;
    // The flattener expands pattern fills whose tile images need flattening, in the layers that print.
    let tile_images = postscript
        && !bitmap
        && super::printadvanced::flattener(PRINT, &set, &s.prefs.flattener_presets)?
            .is_some_and(|o| has_transparent_tile_images(&vectorcraft_pdf::printed_document(doc, &set), &o));
    let level = match p.get("level").filter(|v| postscript && !v.is_null()) {
        Some(v) => {
            let t = super::fileio::dxf::text(v);
            Level::from_id(&t).ok_or_else(|| bad(PRINT, format!("level `{t}`: 2 or 3")))?
        }
        None => Level::default(),
    };
    // A job past `maxPages` is turned away before Print as Bitmap or a flattener preset prepares it,
    // once the options are checked, when its pages follow from the artboards alone (composite, no
    // `skipBlank`, artboards not ignored); the pages are counted again on the prepared art.
    super::printadvanced::flattener(PRINT, &set, &s.prefs.flattener_presets)?;
    let by_artboards = !set.skip_blank
        && !matches!(set.artboards, vectorcraft_pdf::PrintArtboards::Ignore)
        && set.output.mode == vectorcraft_pdf::OutputMode::Composite;
    if max_pages.is_some()
        && by_artboards
        && let Ok(pv) = vectorcraft_pdf::preview(doc, &set)
        && too_many(pv.pages)
    {
        return Ok(json!({ "pages": pv.pages, "format": if postscript { "postscript" } else { "pdf" } }));
    }
    // Print as Bitmap and the flattener preset (Advanced) render and flatten it first.
    let prepared = super::printadvanced::prepare(PRINT, doc, &set, &s.prefs.flattener_presets, s.prefs.black_output == "rich")?;
    let path = str_param(p, "path");
    // The type is read before raster effects and Print as Bitmap turn it into images.
    let which = set.print_layers;
    if !postscript {
        let printed = prepared.as_ref().unwrap_or(doc);
        let laid_out = vectorcraft_pdf::preview(printed, &set).map_err(|e| pdf_error(PRINT, e))?;
        // `maxPages` is checked on the art as it prints, before the job is composed.
        if too_many(laid_out.pages) {
            return Ok(json!({ "pages": laid_out.pages, "format": "pdf" }));
        }
        let typed = type_warnings(&full, which, &areas(&laid_out));
        let mut r =
            vectorcraft_pdf::print(printed, &PrintOptions { settings: set, for_printer: bool_or(p, "forPrinter", false), ..Default::default() })
                .map_err(|e| pdf_error(PRINT, e))?;
        // What the job loses comes first, then the notes on the options.
        r.warnings.extend(typed);
        r.warnings.extend(r.setting_notes);
        return write_or_return(path, &r.bytes, json!({ "pages": r.pages, "format": "pdf", "warnings": r.warnings }));
    }
    // What the job loses, then the notes on the options: a reply lists them in that order.
    let (mut warnings, mut setting_notes) = (vec![], vec![]);
    if !bitmap && (tile_images || doc.layers.iter().any(|l| l.shows_transparency())) {
        warnings.push("transparency is flattened into opaque art and images (PostScript has none): see flattenerPreset".to_string());
    }
    if !set.color.profile.trim().is_empty() && set.output.mode == vectorcraft_pdf::OutputMode::Composite {
        setting_notes.push("the printer profile converts the colours of PDF output: PostScript prints the document's colours".to_string());
    }
    let flatness = (!set.graphics.auto_flatness).then_some(set.graphics.flatness);
    let fonts = match set.graphics.fonts {
        vectorcraft_pdf::FontDownload::None => PrintFonts::Resident,
        vectorcraft_pdf::FontDownload::Subset => PrintFonts::Subset,
        vectorcraft_pdf::FontDownload::Complete => PrintFonts::Complete,
    };
    let plan = vectorcraft_pdf::plan(prepared.as_ref().unwrap_or(doc), &PrintOptions { settings: set, postscript: true, ..Default::default() })
        .map_err(|e| pdf_error(PRINT, e))?;
    if too_many(plan.order.len()) {
        return Ok(json!({ "pages": plan.order.len(), "format": "postscript" }));
    }
    let pages: Vec<PrintPage> = plan
        .sheets
        .iter()
        .map(|sh| PrintPage {
            doc: sh.plate.and_then(|i| plan.plates.get(i)).unwrap_or(&plan.doc),
            size: sh.size,
            view: sh.view,
            window: sh.window,
            place: sh.place,
            copies: &sh.copies,
            area: sh.area,
            marks: sh.marks.as_ref(),
            negative: sh.negative,
            ink: sh.ink.as_ref().map(|i| (i.name.as_str(), i.frequency, i.angle)),
        })
        .collect();
    let job = PrintJob { level, title: plan.title.clone(), created: plan.created, flatness, fonts };
    let out = vectorcraft_eps::print(&pages, &plan.order, &job).map_err(|e| bad(PRINT, e))?;
    for w in plan.warnings.iter().chain(&out.warnings) {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    let areas: Vec<Rect> = plan.sheets.iter().filter_map(|sh| on_page(sh.area, sh.view * sh.place, sh.size)).collect();
    warnings.extend(type_warnings(&full, which, &areas));
    for w in plan.setting_notes.iter().chain(&out.setting_notes) {
        if !warnings.contains(w) && !setting_notes.contains(w) {
            setting_notes.push(w.clone());
        }
    }
    warnings.extend(setting_notes);
    write_or_return(path, &out.bytes, json!({ "pages": plan.order.len(), "format": "postscript", "warnings": warnings }))
}
