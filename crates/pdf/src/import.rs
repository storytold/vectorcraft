//! PDF → Document (hayro-interpret device that builds a VectorCraft node tree).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use craft_pdf::interpret::font::Glyph;
use craft_pdf::interpret::hayro_cmap::BfString;
use craft_pdf::interpret::pattern::{Pattern, TilingPattern};
use craft_pdf::interpret::{
    CacheKey, ClipPath, Context, Device, GlyphDrawMode, Image, ImageData, InterpreterCache, InterpreterSettings, InterpreterWarning, LumaData,
    MaskType, PathDrawMode, SoftMask, StrokeProps,
};
use craft_pdf::syntax::object::Name;
use craft_pdf::syntax::page::Page;
use kurbo::{Affine, BezPath, PathEl, Rect, Shape};
use vectorcraft_color::{BlendMode, Color, Paint, Swatch};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, ColorMode, Dash, Document, FillLayer, ImageBlob, ImageObject, Knockout, LayerColor, LineCap, LineJoin,
    Node, NodeId, NodeKind, OpacityMask, PatternDef, StrokeLayer,
};
use vectorcraft_geom::{FillRule, PathData};

use crate::import_color::{Colors, Native};
use crate::import_mask::{MaskSpec, contains, is_rectangle, luminance, mask_spec, white_cover};
use crate::import_scan::{FontNames, MAX_NESTING, Ocgs, Scan, TextOverprint, Walked, all_on, hides_forms, scan_page, tag_key};
use crate::import_shading::{clipped, extend_clip, fold_stop_opacity, gradient_shape, mesh_shading, shading_gradient};
use crate::import_text::{Families, LineFacts, Look, Placement, TextLine, Upright};
use crate::{CropTo, ImportOptions, ImportReport, PdfError, TextAs};

/// Mixed into an image's cache key when it is read white inside an alpha soft mask.
const WHITE_KEY: u128 = 0x7717_e000_a1fa;
/// The name of the paths text imports as.
const TEXT_OUTLINES: &str = "<Text Outlines>";
/// A page whose overprint (ISO 32000-1 §8.6.7) is left out rather than given to the wrong objects.
const OVERPRINT_UNREAD: &str = "the overprint of a page's objects couldn't be read: they knock out";
/// The document overprints as overprint mode 1 does.
const OVERPRINT_ZERO_MODE: &str =
    "overprint in the mode where a process colour's zero tints knock out (/OPM 0) is imported as overprint, where they don't";
/// The deepest soft masks, patterns and type 3 glyphs are read within each other.
const MAX_NESTED: u32 = 8;
/// A tiling pattern read this many times with the same art is taken to always draw it.
const PATTERN_REUSE: u32 = 16;
/// Stands for the group of art whose group couldn't be told (see [`Builder::unsure`]): it goes
/// to a hidden, non-printing layer of its page.
const UNSORTED: usize = usize::MAX;

/// Import a PDF (or PDF-compatible `.ai`) with default options.
pub fn import(bytes: &[u8]) -> Result<Document, PdfError> {
    import_with_report(bytes, &ImportOptions::default()).map(|r| r.document)
}

/// A layer of the imported document, in paint order.
enum Slot {
    /// A page's art outside optional content groups.
    Page(Box<Node>),
    /// An optional content group (index into [`Ocgs::list`]).
    Group(usize),
}

/// What each group's layer holds, in paint order: its sublayers (their groups) and, where its
/// own group is listed, its art.
type Nesting = HashMap<usize, Vec<usize>>;

/// Group `g` paints (its first art): note where its art goes in its layer, and where that layer
/// goes when it is new: in its parent's layer (placed the same way) or, for a top-level group, in
/// `slots`.
fn place_group(g: usize, ocgs: &Ocgs, slots: &mut Vec<Slot>, nesting: &mut Nesting) {
    let placed = nesting.contains_key(&g);
    nesting.entry(g).or_default().push(g);
    if placed {
        return;
    }
    let mut child = g;
    // Each parent is placed before its sublayers, so the chain ends (within the nesting depth).
    for _ in 0..=MAX_NESTING {
        let Some(parent) = ocgs.list.get(child).and_then(|o| o.parent) else { break };
        let placed = nesting.contains_key(&parent);
        nesting.entry(parent).or_default().push(child);
        if placed {
            return;
        }
        child = parent;
    }
    slots.push(Slot::Group(child));
}

/// The top-level group that group `g` is listed under (`g` itself when it isn't a sublayer).
fn top_group(g: usize, ocgs: &Ocgs) -> usize {
    let mut top = g;
    for _ in 0..=MAX_NESTING {
        match ocgs.list.get(top).and_then(|o| o.parent) {
            Some(parent) => top = parent,
            None => break,
        }
    }
    top
}

/// Merge the top-level groups a page marks (`page`, in the order it first marks each, its empty
/// ones too) into the stacking order of the pages before (`order`, bottom first; `known`: its
/// groups). A group new to it goes right below the first group the page marks after it that it
/// has: a layer whose art starts on a later page keeps its place among the others (#508).
fn merge_order(order: &mut Vec<usize>, known: &mut HashSet<usize>, page: &[usize]) {
    let mut above = None;
    for &g in page.iter().rev() {
        if known.insert(g) {
            let at = above.and_then(|a| order.iter().position(|&x| x == a)).unwrap_or(order.len());
            order.insert(at, g);
        }
        above = Some(g);
    }
}

/// The layer of group `g`, in `color`: its art and sublayers. `None` for a group that isn't read.
fn group_layer(
    b: &mut Builder<'_>,
    g: usize,
    ocgs: &Ocgs,
    nesting: &Nesting,
    art: &mut HashMap<usize, Vec<Arc<Node>>>,
    color: LayerColor,
) -> Option<Node> {
    let ocg = ocgs.list.get(g)?;
    let mut l = Node::layer(b.id(), &ocg.name, color);
    (l.visible, l.locked) = (ocg.on, ocg.locked);
    let mut children = vec![];
    for &c in nesting.get(&g).into_iter().flatten() {
        if c == g {
            children.extend(art.remove(&g).unwrap_or_default());
        } else if let Some(sub) = group_layer(b, c, ocgs, nesting, art, color) {
            children.push(Arc::new(sub));
        }
    }
    if let NodeKind::Layer { children: c, printable, .. } = &mut l.kind {
        *c = children;
        *printable = ocg.print;
    }
    Some(l)
}

/// The note on a file carrying an editor's private data (#472): its PDF part, the one read, holds
/// only the art on its artboards (art on the pasteboard is in the private data alone).
pub const OFF_ARTBOARD_NOTE: &str =
    "only the PDF-compatible part of this file was read: art outside its artboards is kept in the editor's private data alone, so it doesn't open";

/// Warning: art reaching past the page box was clipped to it.
pub const PAST_PAGE_NOTE: &str = "art reaching past the page was clipped to it, as PDF viewers show it";

/// Import a PDF, returning the document plus warnings about content that was approximated or skipped.
pub fn import_with_report(bytes: &[u8], opts: &ImportOptions) -> Result<ImportReport, PdfError> {
    // The interpreter draws forms, patterns, glyphs and masks inside one another by recursion, in
    // large stack frames: it caps that (50 deep), and gets room for the cap whatever thread asks.
    vectorcraft_doc::depth::with_deep_stack(|| read(bytes, opts))
}

/// [`import_with_report`] on the thread that asks.
pub(crate) fn read(bytes: &[u8], opts: &ImportOptions) -> Result<ImportReport, PdfError> {
    read_with_reader_options(bytes, opts, &crate::pages::open_options())
}

#[cfg(test)]
pub(crate) fn import_with_reader_options(bytes: &[u8], opts: &ImportOptions, reader: &craft_pdf::OpenOptions) -> Result<ImportReport, PdfError> {
    vectorcraft_doc::depth::with_deep_stack(|| read_with_reader_options(bytes, opts, reader))
}

fn read_with_reader_options(bytes: &[u8], opts: &ImportOptions, reader: &craft_pdf::OpenOptions) -> Result<ImportReport, PdfError> {
    let original = crate::pages::open_with_options(bytes, opts.password.as_deref(), reader)?;
    let picked = crate::pages::picked(opts, original.pages().len())?;
    let mut ocgs = Ocgs::read(&original);
    let route = opts.layers && !ocgs.list.is_empty();
    // Content that is off imports as hidden layers: an update of the file turns every group on
    // (unless a form is hidden by its own group: its art couldn't be told apart).
    let hide = route && ocgs.skips_any();
    let mut hidden_form = false;
    if hide {
        for page in picked.iter().filter_map(|&n| original.pages().get(n)) {
            hidden_form |= hides_forms(page, &mut ocgs, original.decode_limits().max_decoded_stream())?;
            if hidden_form {
                break;
            }
        }
    }
    let turned_on = (hide && !hidden_form)
        .then(|| all_on(bytes, &original))
        .flatten()
        .and_then(|b| crate::pages::open_with_options(&b, opts.password.as_deref(), reader).ok())
        .filter(|p| Ocgs::read(p).list.is_empty());
    let mut notes = vec![];
    if hide && turned_on.is_none() {
        notes.push("the art of hidden layers couldn't be read and was left out".to_string());
    }
    let all_on = turned_on.is_some();
    let pdf = turned_on.unwrap_or(original);
    let pages = pdf.pages();

    let sink: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink2 = sink.clone();
    let incomplete = Arc::new(AtomicBool::new(false));
    let incomplete_sink = incomplete.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |w| {
            let msg = match w {
                InterpreterWarning::UnsupportedFont => "a font could not be read; its text was skipped",
                InterpreterWarning::ImageDecodeFailure => "an image could not be decoded and was skipped",
                InterpreterWarning::ContentTruncated => {
                    incomplete_sink.store(true, Ordering::Relaxed);
                    return;
                }
            };
            if let Ok(mut v) = sink2.lock()
                && !v.iter().any(|x| x == msg)
            {
                v.push(msg.to_string());
            }
        }),
        render_annotations: false,
        ..Default::default()
    };

    let mut doc = Document::new(1.0, 1.0);
    doc.title = "Imported PDF".into();
    doc.artboards.clear();
    doc.layers.clear();
    let taken: Vec<String> = doc.swatches_iter().map(|s| s.name.clone()).chain(doc.swatch_groups.iter().map(|g| g.name.clone())).collect();
    let mut b = Builder::new(doc.peek_next_id(), Colors::new(&pdf, taken.clone()), opts.text_as, route);
    // The art of a group that is off (drawn as `all_on` turned it on) or doesn't print mustn't
    // show or print on its page's layer.
    b.unsure = route && ocgs.list.iter().any(|g| (all_on && !g.on) || !g.print);
    b.taken = taken;
    b.warnings = notes;
    b.colors.uncalibrated_rgb = opts.uncalibrated_rgb && picked.iter().filter_map(|&n| pages.get(n)).any(crate::pages::has_private_data);
    let cache = InterpreterCache::new();
    let mut x = 0.0;
    // Where the next page's art may begin (a `.ai` page's art reaches past its artboard).
    let mut clear = f64::NEG_INFINITY;
    // Every page only text over private data: a file saved without its PDF part shows a notice
    // there, not its art. A page that states where its art is isn't that notice.
    let (mut only_text, mut art_boxes) = (true, false);
    let mut slots: Vec<Slot> = vec![];
    let mut group_art: HashMap<usize, Vec<Arc<Node>>> = HashMap::new();
    let mut nesting = Nesting::new();
    // The top-level groups in the order the pages mark them, bottom first.
    let (mut order, mut known) = (vec![], HashSet::new());
    for (i, &number) in picked.iter().enumerate() {
        let Some(page) = pages.get(number) else { continue };
        // Artboards on the trim boxes have the bleed boxes round them as the document's bleed
        // (the widest of the pages' on each side), so the art there shows and prints as bleed.
        if opts.crop == CropTo::Trim {
            for (side, b) in doc.setup.bleed.iter_mut().zip(crate::pages::bleed(page)) {
                *side = side.max(b);
            }
        }
        // The chosen box sits at (x, 0); the page draws round it.
        let (init, frame) = crate::pages::frame(page, opts.crop);
        let mut ab = Rect::new(x, 0.0, x + frame.width(), frame.height());
        let xf = Affine::translate((x - frame.x0, -frame.y0)) * init;
        let mut ctx = Context::new(xf, ab, &cache, pdf.xref(), settings.clone());
        b.page = ab;
        // The first id of this page's art.
        let first_id = b.next;
        let mut scan = scan_page(page, &mut ocgs, all_on, &mut b.fonts, pdf.decode_limits().max_decoded_stream())?;
        // The walk's ink names pick between spaces of inks that look the same to the interpreter. A
        // page keeps them only when it uses such an ink and the interpreter draws as many paths
        // there as the walk counts.
        let tied = scan.inks.iter().any(|n| b.colors.tied(n));
        if !(tied && drawn_paths(page, Context::new(xf, ab, &cache, pdf.xref(), settings.clone())) == Some(scan.paints.len())) {
            scan.inks.clear();
        }
        if route {
            let mut seen = HashSet::new();
            let marked: Vec<usize> = scan.tags.iter().filter_map(|(_, g)| g.map(|g| top_group(g, &ocgs))).filter(|g| seen.insert(*g)).collect();
            merge_order(&mut order, &mut known, &marked);
        }
        b.begin_page(scan);
        craft_pdf::interpret_page(page, &mut ctx, &mut b).map_err(|e| PdfError::Parse(e.to_string()))?;
        if incomplete.load(Ordering::Relaxed) {
            return Err(crate::import_scan::incomplete_content());
        }
        let mut parts = b.end_page();
        // A PDF-compatible `.ai` (the editor's private data next to the PDF art).
        let ai = crate::pages::has_private_data(page);
        if ai {
            drop_page_fill(&mut parts, xf.transform_rect_bbox(crate::pages::page_box(page, CropTo::Crop)));
            b.warn(OFF_ARTBOARD_NOTE);
        }
        // A file with several artboards writes, on each page, the art of its neighbours that
        // reaches into the page's box: art lying wholly outside this page is theirs (each page
        // draws it shifted by its own artboard spacing), so keep it only where it belongs.
        if picked.len() > 1 {
            for (_, art) in &mut parts {
                art.retain(|n| n.visual_bounds().is_none_or(|r| r.x0 <= ab.x1 && r.x1 >= ab.x0 && r.y0 <= ab.y1 && r.y1 >= ab.y0));
            }
            parts.retain(|(_, art)| !art.is_empty());
        }
        // Viewers show a page through its crop box, so art reaching past it (a mock-up backdrop
        // around the page, say) never showed. A clip to the page box that the file states is
        // dropped as redundant (by hayro and by `push_clip_path`), so restore the box here:
        // each object reaching past it keeps its own clip, which Fit to Artwork Bounds and
        // `CropTo::Bounding` then respect.
        // The crop box is what viewers show, whichever box the artboard takes. Not in a
        // PDF-compatible `.ai`: its PDF part stands in for the editable document, whose art
        // crossing the artboard edge isn't clipped, so it opens whole (#646).
        let shown = xf.transform_rect_bbox(crate::pages::page_box(page, CropTo::Crop));
        let page_box = shown.inflate(0.01, 0.01);
        let mut clipped = false;
        for (_, art) in parts.iter_mut().filter(|_| !ai) {
            for n in art.iter_mut() {
                if n.visual_bounds().is_some_and(|r| !contains(page_box, r)) {
                    let clip = Arc::new(b.clip_node(&shown.to_path(0.1), FillRule::NonZero));
                    *n = Arc::new(Node::new(b.id(), NodeKind::Group { children: vec![clip, n.clone()], clip: true }));
                    clipped = true;
                }
            }
        }
        if clipped {
            b.warn(PAST_PAGE_NOTE);
        }
        // A `.ai` page keeps its art past the artboard edge, unclipped (#646), so a page whose art
        // reaches far to the left would lay it over the page before it in the row: there it would
        // be taken for that page's art (by the check against the editing data, which then finds
        // art its layers don't have there, and by `place_pages`). Such a page moves right until its
        // art clears the art before it.
        let mut art_right = f64::NEG_INFINITY;
        if ai && let Some(art) = parts.iter().flat_map(|(_, v)| v.iter()).filter_map(|n| n.visual_bounds()).reduce(|a, b| a.union(b)) {
            // Art at a coordinate that isn't a number (a damaged file) moves nothing.
            let dx = Some((clear - art.x0).max(0.0)).filter(|dx| dx.is_finite()).unwrap_or(0.0);
            if dx > 0.0 {
                let shift = Affine::translate((dx, 0.0));
                for (_, v) in &mut parts {
                    for n in v.iter_mut() {
                        Arc::make_mut(n).transform(shift, false);
                    }
                }
                // What the page's lines of type were is told where their glyphs are (the outlines
                // stroked on them are looked for there), so that moves with them.
                for facts in b.lines.iter_mut().filter(|(id, _)| id.0 >= first_id).map(|(_, facts)| facts) {
                    for ink in &mut facts.ink {
                        *ink = *ink + kurbo::Vec2::new(dx, 0.0);
                    }
                }
                ab = Rect::new(ab.x0 + dx, ab.y0, ab.x1 + dx, ab.y1);
            }
            art_right = art.x1 + dx;
        }
        let children: Vec<Arc<Node>> = parts.iter().flat_map(|(_, v)| v.iter().cloned()).collect();
        only_text &= ai && is_only_text(&children);
        art_boxes |= crate::pages::states_art(page);
        let mut right = ab.x1;
        if opts.crop == CropTo::Bounding
            && let Some(art) = vectorcraft_doc::live::nodes_bounds(&children)
        {
            right = right.max(art.x1);
            ab = art;
        }
        x = right + opts.artboard_gap;
        clear = right.max(art_right) + opts.artboard_gap;
        doc.artboards.push(Artboard {
            id: i as u32 + 1,
            name: format!("Artboard {}", i + 1),
            rect: ab,
            show_center_mark: false,
            show_cross_hairs: false,
            color: None,
        });
        let page_layer = |b: &mut Builder<'_>, art: Vec<Arc<Node>>, shown: bool| {
            let name = if shown { format!("Page {}", number + 1) } else { format!("Page {} (unsorted)", number + 1) };
            let mut layer = Node::layer(b.id(), &name, LayerColor::Preset((i % 27) as u8));
            layer.visible = shown;
            if let NodeKind::Layer { children: c, printable, .. } = &mut layer.kind {
                (*c, *printable) = (art, shown);
            }
            Slot::Page(Box::new(layer))
        };
        if parts.is_empty() {
            slots.push(page_layer(&mut b, vec![], true));
        }
        for (key, art) in parts {
            match key {
                None => slots.push(page_layer(&mut b, art, true)),
                Some(UNSORTED) => slots.push(page_layer(&mut b, art, false)),
                Some(g) => {
                    let into = group_art.entry(g).or_insert_with(|| {
                        place_group(g, &ocgs, &mut slots, &mut nesting);
                        vec![]
                    });
                    into.extend(art);
                }
            }
        }
    }
    if only_text && !art_boxes {
        return Err(PdfError::PlaceholderOnly);
    }
    // A group the pages mark but draw nothing in is an empty layer, as the editor had it.
    for &g in &order {
        if !nesting.contains_key(&g) {
            slots.push(Slot::Group(g));
        }
    }
    // The groups' layers stack as the pages mark them (the art outside them stays where it was
    // drawn): a group that first paints on a later page doesn't go on top of the others.
    let rank: HashMap<usize, usize> = order.iter().enumerate().map(|(i, &g)| (g, i)).collect();
    let at: Vec<usize> = slots.iter().enumerate().filter(|(_, s)| matches!(s, Slot::Group(_))).map(|(i, _)| i).collect();
    let mut stacked: Vec<usize> = slots.iter().filter_map(|s| if let Slot::Group(g) = s { Some(*g) } else { None }).collect();
    stacked.sort_by_key(|g| rank.get(g).copied().unwrap_or(usize::MAX));
    for (i, g) in at.into_iter().zip(stacked) {
        if let Some(s) = slots.get_mut(i) {
            *s = Slot::Group(g);
        }
    }
    for (n, slot) in slots.into_iter().enumerate() {
        let mut layer = match slot {
            Slot::Page(l) => *l,
            // Sublayers take their top-level layer's colour.
            Slot::Group(g) => match group_layer(&mut b, g, &ocgs, &nesting, &mut group_art, LayerColor::Preset((n % 27) as u8)) {
                Some(l) => l,
                None => continue,
            },
        };
        if let Some(art) = layer.children_mut() {
            crate::import_lines::rebuild(art, &b.lines);
        }
        doc.layers.push(Arc::new(layer));
    }
    for (k, blob) in b.images.drain() {
        doc.images.insert(k, blob);
    }
    // A file painted mostly in CMYK opens as a CMYK document (with CMYK default swatches).
    if b.colors.cmyk_document() {
        doc.color_mode = ColorMode::Cmyk;
        let (swatches, groups) = vectorcraft_color::default_swatches(ColorMode::Cmyk.model());
        (doc.swatches, doc.swatch_groups) = (swatches.into(), groups.into());
    }
    doc.swatches.extend(b.colors.swatches());
    // Tiling patterns are pattern swatches.
    for p in b.patterns.drain(..) {
        doc.swatches.push(Swatch {
            name: p.name.clone(),
            paint: Paint::Pattern { pattern: p.name.clone(), xf: Affine::IDENTITY },
            global: false,
            spot: false,
        });
        doc.patterns.push(p);
    }
    if b.colors.mixed {
        b.warn("colours mixing several inks (DeviceN) were imported as RGB");
    }
    if !b.missing_fonts.is_empty() {
        let list = b.missing_fonts.join(", ");
        b.warn(&format!("fonts that aren't available show in the fallback font until they are: {list}"));
    }
    doc.fix_next_id();
    doc.reserve_ids(b.next);

    let mut warnings = b.warnings;
    if let Ok(v) = sink.lock() {
        for w in v.iter() {
            if !warnings.contains(w) {
                warnings.push(w.clone());
            }
        }
    }
    doc.limit_depth_noting(&mut warnings);
    let user_unit = picked.first().and_then(|&n| pages.get(n)).map_or(1.0, |page| f64::from(page.user_unit()));
    Ok(ImportReport { document: doc, warnings, native: crate::editing::editing_in(&pdf), only_text, user_unit })
}

/// How many paths the interpreter draws on `page` (each fill and each stroke), leaving out those
/// of soft masks, pattern cells and type 3 glyphs, which [`Builder`] reads nested. `None` when the
/// interpreter fails.
fn drawn_paths<'a>(page: &'a Page<'a>, mut ctx: Context<'a>) -> Option<usize> {
    struct Count(usize);
    impl<'a> Device<'a> for Count {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
        fn set_blend_mode(&mut self, _: craft_pdf::interpret::BlendMode) {}
        // As `Builder::draw_path` takes them: each fill and each stroke.
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &craft_pdf::interpret::Paint<'a>, _: &PathDrawMode) {
            self.0 += 1;
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: craft_pdf::interpret::BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'a>, _: Affine, _: Affine, _: &craft_pdf::interpret::Paint<'a>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let mut count = Count(0);
    craft_pdf::interpret_page(page, &mut ctx, &mut count).ok()?;
    Some(count.0)
}

/// The characters of a CID-keyed CFF font embedded without a ToUnicode map (as macOS writes
/// Hiragino): each glyph's CID from the font's charset, then Adobe's CID → Unicode table of its
/// character collection (Japan1, GB1, CNS1, Korea1).
pub(crate) struct CidText {
    /// CID of each glyph.
    cids: Vec<u16>,
    table: craft_pdf::interpret::hayro_cmap::CMap,
}

impl CidText {
    pub(crate) fn of(data: &[u8]) -> Option<Self> {
        // An embedded CFF of a few CJK glyphs is kilobytes; whole fonts are some megabytes.
        if data.len() > 64 << 20 {
            return None;
        }
        use craft_pdf::interpret::hayro_cmap::{CMap, CMapName, load_embedded};
        use skrifa::raw::ps::cff::{CffFontRef, dict, v1::Cff};
        use skrifa::raw::{FontData, FontRead};
        let cff = Cff::read(FontData::new(data)).ok()?;
        let top = cff.top_dicts().get(0)?;
        let ordering = dict::entries(top, None).filter_map(Result::ok).find_map(|e| match e {
            dict::Entry::Ros { ordering, .. } => Some(ordering),
            _ => None,
        })?;
        let name = match cff.string(ordering)? {
            b"Japan1" => CMapName::AdobeJapan1Ucs2,
            b"GB1" => CMapName::AdobeGb1Ucs2,
            b"CNS1" => CMapName::AdobeCns1Ucs2,
            b"Korea1" => CMapName::AdobeKorea1Ucs2,
            _ => return None,
        };
        let table = CMap::parse(load_embedded(name)?, load_embedded)?;
        let charset = CffFontRef::new(data, 0, None).ok()?.charset()?;
        let cids = (0..charset.num_glyphs().min(65_536)).map(|g| charset.string_id(skrifa::GlyphId::new(g)).map_or(0, |s| s.to_u16())).collect();
        Some(Self { cids, table })
    }

    fn unicode(&self, glyph: u32) -> Option<craft_pdf::interpret::hayro_cmap::BfString> {
        let cid = *self.cids.get(glyph as usize)?;
        self.table.lookup_bf_string(u32::from(cid))
    }
}

/// A ToUnicode value that names no character: U+FFFD alone (#708).
fn replacement(u: &craft_pdf::interpret::hayro_cmap::BfString) -> bool {
    use craft_pdf::interpret::hayro_cmap::BfString;
    match u {
        BfString::Char(c) => *c == char::REPLACEMENT_CHARACTER,
        BfString::String(s) => !s.is_empty() && s.chars().all(|c| c == char::REPLACEMENT_CHARACTER),
    }
}

/// The character of each glyph of an embedded TrueType or OpenType font program, from its own
/// `cmap` (the first character mapped to each glyph); `None` for a bare CFF or a font without one.
pub(crate) fn font_chars(data: &[u8]) -> Option<HashMap<u32, char>> {
    use skrifa::MetadataProvider;
    // As for CidText: whole fonts are some megabytes.
    if data.len() > 64 << 20 {
        return None;
    }
    let font = skrifa::FontRef::new(data).ok()?;
    let mut chars = HashMap::new();
    for (c, g) in font.charmap().mappings() {
        if let Some(c) = char::from_u32(c).filter(|c| !c.is_control()) {
            chars.entry(g.to_u32()).or_insert(c);
        }
    }
    (!chars.is_empty()).then_some(chars)
}

/// The ideographs of the Kangxi radicals U+2F00–U+2FD5 (their NFKC forms): the CID → Unicode
/// tables of some PDFs map 龍 to the radical ⿓, which looks the same but isn't the character.
const KANGXI: &str = "一丨丶丿乙亅二亠人儿入八冂冖冫几凵刀力勹匕匚匸十卜卩厂厶又口囗土士夂夊夕大女子宀寸小尢尸屮山巛工己巾干幺广廴廾弋弓彐彡彳心戈戶手支攴文斗斤方无日曰月木欠止歹殳毋比毛氏气水火爪父爻爿片牙牛犬玄玉瓜瓦甘生用田疋疒癶白皮皿目矛矢石示禸禾穴立竹米糸缶网羊羽老而耒耳聿肉臣自至臼舌舛舟艮色艸虍虫血行衣襾見角言谷豆豕豸貝赤走足身車辛辰辵邑酉釆里金長門阜隶隹雨靑非面革韋韭音頁風飛食首香馬骨高髟鬥鬯鬲鬼魚鳥鹵鹿麥麻黃黍黑黹黽鼎鼓鼠鼻齊齒龍龜龠";

/// How far glyph `o` (the PDF's, for `text`) is from the installed `face`'s glyph for it: the
/// largest difference between their boxes, in thousandths of an em. `None` when they are clearly
/// different glyphs (or the face lacks the character); `Some(0)` when there is nothing to compare
/// (several characters, a vertical form, no ink in either).
fn glyph_deviation(face: &vectorcraft_text::FontFace, text: &str, o: &craft_pdf::interpret::font::OutlineGlyph, vertical: bool) -> Option<f64> {
    let Some(c) = comparable(text, vertical) else { return Some(0.0) };
    let gid = face.glyph_for(c);
    if gid == 0 {
        return None;
    }
    let embedded = o.outline();
    let installed = vectorcraft_text::FontDb::global().outline(face, gid);
    match (embedded.elements().is_empty(), installed.elements().is_empty()) {
        (true, true) => Some(0.0),
        (false, false) => {
            let e = embedded.bounding_box();
            let k = 1000.0 / face.units_per_em();
            // Installed outlines are y-down; the PDF's glyph space is y-up.
            let i = installed.bounding_box();
            let i = Rect::new(i.x0 * k, -i.y1 * k, i.x1 * k, -i.y0 * k);
            let off = [(e.x0, i.x0), (e.x1, i.x1), (e.y0, i.y0), (e.y1, i.y1)].iter().fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
            (off <= 100.0).then_some(off)
        }
        _ => None,
    }
}

/// The face of `face`'s family that draws glyph `o` (for `text`): `face` itself, or the style whose
/// glyph is closest (a variable font's named instances, or a family whose PostScript names don't
/// tell its styles apart, draw a character differently in each style). `None`: no style of the
/// family has that glyph, so the font isn't the installed one.
fn matching_face(
    face: &Arc<vectorcraft_text::FontFace>,
    text: &str,
    o: &craft_pdf::interpret::font::OutlineGlyph,
    vertical: bool,
) -> Option<Arc<vectorcraft_text::FontFace>> {
    let own = glyph_deviation(face, text, o, vertical);
    if own == Some(0.0) {
        return Some(face.clone());
    }
    let db = vectorcraft_text::FontDb::global();
    // Other installed versions of its family and style (a font library keeps old ones), then the
    // family's other styles.
    let versions = db.versions(face).into_iter().skip(1).take(16);
    let styles =
        db.styles(&face.family).into_iter().filter(|s| !s.eq_ignore_ascii_case(&face.style)).take(64).filter_map(|s| db.face(&face.family, &s));
    let others = versions.chain(styles);
    let mut best = own.map(|d| (d, face.clone()));
    for f in others {
        if let Some(d) = glyph_deviation(&f, text, o, vertical)
            && best.as_ref().is_none_or(|(b, _)| d < *b)
        {
            best = Some((d, f));
        }
    }
    best.map(|(_, f)| f)
}

/// The characters a glyph no mapping names is looked for among (#811): those single-byte fonts
/// encode (ASCII, Latin-1, Latin Extended-A, general punctuation, the euro and trade mark signs,
/// the fi and fl ligatures).
const IDENTIFIABLE: [std::ops::RangeInclusive<u32>; 6] =
    [0x21..=0x7E, 0xA1..=0x17F, 0x2010..=0x2044, 0x20AC..=0x20AC, 0x2122..=0x2122, 0xFB01..=0xFB02];

/// The character whose glyph in installed `face` draws `embedded` (a glyph outline in thousandths
/// of an em, y up): the first of [`IDENTIFIABLE`] with the same outline, point for point. A subset
/// font embedded without a character map still carries the font's own outlines, so a glyph whose
/// subset code names nothing is found this way. `None` when no glyph matches (another version of
/// the font, or a glyph that is no character), and for empty glyphs, which every space draws.
pub(crate) fn identify_glyph(face: &vectorcraft_text::FontFace, embedded: &BezPath) -> Option<char> {
    if embedded.elements().is_empty() {
        return None;
    }
    let k = 1000.0 / face.units_per_em();
    // Installed outlines are y-down in font units.
    let to_pdf = Affine::new([k, 0.0, 0.0, -k, 0.0, 0.0]);
    let db = vectorcraft_text::FontDb::global();
    IDENTIFIABLE.iter().cloned().flatten().filter_map(char::from_u32).find(|c| {
        let gid = face.glyph_for(*c);
        gid != 0 && same_shape(embedded, &(to_pdf * db.outline(face, gid).as_ref().clone()))
    })
}

/// Do outlines `a` and `b` draw the same segments, every point within 2/1000 em (closing
/// commands aside)?
fn same_shape(a: &BezPath, b: &BezPath) -> bool {
    let drawn = |p: &BezPath| p.elements().iter().filter(|e| !matches!(e, PathEl::ClosePath)).copied().collect::<Vec<_>>();
    let (a, b) = (drawn(a), drawn(b));
    let near = |p: kurbo::Point, q: kurbo::Point| (p - q).hypot() <= 2.0;
    a.len() == b.len()
        && a.iter().zip(&b).all(|(x, y)| match (x, y) {
            (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => near(*p, *q),
            (PathEl::QuadTo(p1, p2), PathEl::QuadTo(q1, q2)) => near(*p1, *q1) && near(*p2, *q2),
            (PathEl::CurveTo(p1, p2, p3), PathEl::CurveTo(q1, q2, q3)) => near(*p1, *q1) && near(*p2, *q2) && near(*p3, *q3),
            _ => false,
        })
}

/// The character of glyph `text` when its outline can be compared with an installed font's: a
/// single character, and not CJK punctuation set vertically (its vertical form differs).
fn comparable(text: &str, vertical: bool) -> Option<char> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    if vertical && matches!(c as u32, 0x2014 | 0x2015 | 0x2025 | 0x2026 | 0x3000..=0x303F | 0x30FC | 0xFE30..=0xFE4F | 0xFF00..=0xFF65) {
        return None;
    }
    Some(c)
}

pub(crate) fn unify_radical(c: char) -> char {
    let i = (c as u32).wrapping_sub(0x2F00);
    if i < 214 { KANGXI.chars().nth(i as usize).unwrap_or(c) } else { c }
}

/// The font a PDF font draws with ([`Builder::font_name`]).
#[derive(Clone)]
struct FontInfo {
    family: String,
    style: String,
    /// The installed face of its PostScript name.
    face: Option<Arc<vectorcraft_text::FontFace>>,
}

/// The page a PDF-compatible `.ai` paints under its layers is the editor's page, not art: an opaque
/// white rectangle the size of the page (`page`, document space), outside every layer and before
/// any of them. Drop it from the page's art (`parts`, as [`Builder::end_page`] gives them).
fn drop_page_fill(parts: &mut Vec<(Option<usize>, Vec<Arc<Node>>)>, page: Rect) {
    let layered = parts.iter().any(|(g, _)| g.is_some());
    let Some((None, art)) = parts.first_mut() else { return };
    // Not a spot colour: a white ink is art.
    let fill = art.first().is_some_and(|n| {
        layered
            && n.blend == BlendMode::Normal
            && white_cover(n, page)
            && n.appearance.fill().is_some_and(|f| matches!(f.paint, Paint::Solid { swatch: None, .. }))
            && n.geometric_bounds().is_some_and(|b| contains(page.inflate(0.5, 0.5), b))
    });
    if fill {
        art.remove(0);
        if art.is_empty() {
            parts.remove(0);
        }
    }
}

/// Is this art text and nothing else (in groups and clips), with some text?
fn is_only_text(nodes: &[Arc<Node>]) -> bool {
    fn walk(nodes: &[Arc<Node>], text: &mut bool) -> bool {
        nodes.iter().all(|n| match &n.kind {
            NodeKind::Path { clipping: true, .. } => true,
            NodeKind::Path { .. } if n.name.as_deref() == Some(TEXT_OUTLINES) => {
                *text = true;
                true
            }
            NodeKind::Text(_) => {
                *text = true;
                true
            }
            NodeKind::Group { children, .. } => walk(children, text),
            _ => false,
        })
    }
    let mut text = false;
    walk(nodes, &mut text) && text
}

/// Decoded RGBA pixels, width, height and hayro's scale factors.
type Decoded = (Vec<u8>, u32, u32, (f32, f32));

enum FrameKind {
    Root,
    /// A clip group whose clip path node is stored here.
    Clip(Box<Node>),
    /// A clip that was redundant (covers the page); children go straight to the parent.
    Skip,
    Group {
        opacity: f32,
        blend: BlendMode,
        mask: Option<Arc<MaskSpec>>,
        isolate: bool,
        knockout: bool,
    },
}

struct Frame {
    kind: FrameKind,
    children: Vec<Arc<Node>>,
    /// The first optional content group entered inside (where the frame's art goes).
    group: Option<usize>,
    /// The group each child was drawn in, where the frame's art goes when the frame dissolves
    /// into its parent (a layer's form drawing its sublayers' forms).
    child_groups: Vec<Option<usize>>,
}

impl Frame {
    fn new(kind: FrameKind) -> Self {
        Self { kind, children: vec![], group: None, child_groups: vec![] }
    }
}

/// A dissolving frame's `children`, each with the group it was drawn in (`groups`, else the
/// frame's `group`).
fn grouped(children: Vec<Arc<Node>>, groups: Vec<Option<usize>>, group: Option<usize>) -> impl Iterator<Item = (Arc<Node>, Option<usize>)> {
    children.into_iter().zip(groups.into_iter().map(move |g| g.or(group)).chain(std::iter::repeat(group)))
}

/// Glyphs drawn consecutively with the same paint, merged into one path.
struct GlyphRun {
    path: BezPath,
    paint: Paint,
    opacity: f32,
    stroke: Option<StrokeProps>,
    scale: f64,
    overprint: bool,
}

/// How often a tiling pattern was read, and the art (as its paints) each reading drew with the
/// swatch made for it.
type PatternReadings = (u32, Vec<(Vec<Paint>, String)>);

/// Builder state a nested interpretation (soft mask, pattern cell) starts afresh and gives back.
struct Saved {
    stack: Vec<Frame>,
    root_groups: Vec<Option<usize>>,
    blend: BlendMode,
    mask: Option<Arc<MaskSpec>>,
    page: Rect,
    pending: bool,
}

struct Builder<'p> {
    colors: Colors<'p>,
    next: u64,
    stack: Vec<Frame>,
    /// The optional content group of each child of the root frame.
    root_groups: Vec<Option<usize>>,
    blend: BlendMode,
    page: Rect,
    glyphs: Option<GlyphRun>,
    text: Option<TextLine>,
    text_as: TextAs,
    images: HashMap<String, ImageBlob>,
    image_keys: HashMap<u128, (String, u32, u32)>,
    /// The images kept with their CMYK samples ([`crate::import_image`]).
    cmyk_keys: HashSet<u128>,
    /// Mask keys of CMYK images whose mask turned out to hide nothing.
    opaque: HashSet<u128>,
    warnings: Vec<String>,
    /// Fonts by cache key → base font name and whether the file carries its glyphs (from
    /// [`scan_page`]).
    fonts: FontNames,
    font_names: HashMap<u128, FontInfo>,
    families: Option<Families>,
    missing_fonts: Vec<String>,
    /// Font (cache key) → its characters by glyph, for CID-keyed fonts embedded without a
    /// ToUnicode map.
    cid_text: HashMap<u128, Option<Arc<CidText>>>,
    /// Font (cache key) → the character of each glyph its embedded font program maps
    /// ([`font_chars`]), for glyphs the file's ToUnicode map doesn't name (#708).
    font_chars: HashMap<u128, Option<Arc<HashMap<u32, char>>>>,
    /// Font (cache key) → the installed face that draws its glyphs, decided from its first glyph
    /// that can be compared ([`matching_face`]); `None`: its glyphs differ, so it stays outlines.
    matched: HashMap<u128, Option<Arc<vectorcraft_text::FontFace>>>,
    /// (Font, glyph) → the character its outline matches in the installed face, for glyphs no
    /// mapping names ([`Builder::identified_char`]).
    identified: HashMap<(u128, u32), Option<char>>,
    /// Per font: the installed version its type names, when it isn't the one its family and style
    /// resolve to (see `CharStyle::font_version`).
    versions: HashMap<u128, Option<String>>,
    /// The soft mask of the graphics state, the art drawn through it so far, and the masks read.
    mask: Option<Arc<MaskSpec>>,
    masked: Vec<Arc<Node>>,
    masks: HashMap<u128, Arc<MaskSpec>>,
    /// Pattern swatches made, and per (pattern, stroke) the art signatures read and their names.
    patterns: Vec<PatternDef>,
    pattern_keys: HashMap<(u128, bool), PatternReadings>,
    /// Swatch names in use.
    taken: Vec<String>,
    /// The page as walked by [`scan_page`], and how far the interpreter is through it.
    scan: Scan,
    tag_at: usize,
    group_at: usize,
    /// The fills and strokes of paths drawn so far ([`Scan::paints`]).
    paint_at: usize,
    aligned: bool,
    /// Whether optional content groups become layers.
    route: bool,
    /// Some groups are hidden or don't print: art in marked content whose group can't be told
    /// (once the scan and the interpreter part) is [`UNSORTED`] rather than the page's.
    unsure: bool,
    /// The marked-content sequences open: the group each marks.
    marked: Vec<Option<usize>>,
    /// A transparency group was just pushed: a form's (whose flags come next) or an image's.
    pending: bool,
    nested: u32,
    /// Reading an alpha soft mask's art: images are read white with their own alpha, so the
    /// mask's luminance is their alpha, as whitened vector art's is (cairo, and so Inkscape,
    /// rasterizes masks into such images).
    white_images: bool,
    /// The last filled path: its node and its outline (a stroke of the same outline joins it).
    last_fill: Option<(NodeId, BezPath)>,
    /// What each line of type made tells beyond its text object ([`crate::import_lines`]).
    lines: HashMap<NodeId, LineFacts>,
}

fn blend(b: craft_pdf::interpret::BlendMode) -> BlendMode {
    use craft_pdf::interpret::BlendMode as H;
    match b {
        H::Normal => BlendMode::Normal,
        H::Multiply => BlendMode::Multiply,
        H::Screen => BlendMode::Screen,
        H::Overlay => BlendMode::Overlay,
        H::Darken => BlendMode::Darken,
        H::Lighten => BlendMode::Lighten,
        H::ColorDodge => BlendMode::ColorDodge,
        H::ColorBurn => BlendMode::ColorBurn,
        H::HardLight => BlendMode::HardLight,
        H::SoftLight => BlendMode::SoftLight,
        H::Difference => BlendMode::Difference,
        H::Exclusion => BlendMode::Exclusion,
        H::Hue => BlendMode::Hue,
        H::Saturation => BlendMode::Saturation,
        H::Color => BlendMode::Color,
        H::Luminosity => BlendMode::Luminosity,
    }
}

fn fill_rule(r: craft_pdf::interpret::FillRule) -> FillRule {
    match r {
        craft_pdf::interpret::FillRule::NonZero => FillRule::NonZero,
        craft_pdf::interpret::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

/// `p` with every subpath closed: how a fill reads it.
pub(crate) fn closed_subpaths(p: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in p.elements() {
        if open && matches!(el, PathEl::MoveTo(_)) {
            out.push(PathEl::ClosePath);
        }
        out.push(*el);
        open = !matches!(el, PathEl::ClosePath);
    }
    if open {
        out.push(PathEl::ClosePath);
    }
    out
}

/// Do a fill of `fill` and a stroke of `stroke` paint one object's outline? Applications write the
/// fill without closing the path (filling closes it) and the stroke closed.
fn same_outline(fill: &BezPath, stroke: &BezPath) -> bool {
    fill.elements() == stroke.elements() || closed_subpaths(fill).elements() == closed_subpaths(stroke).elements()
}

fn mean_scale(a: Affine) -> f64 {
    a.determinant().abs().sqrt()
}

fn stroke_layer(paint: Paint, opacity: f32, p: &StrokeProps, scale: f64) -> StrokeLayer {
    let mut st = StrokeLayer::new(paint, p.line_width as f64 * scale);
    st.opacity = opacity;
    st.cap = match p.line_cap {
        kurbo::Cap::Butt => LineCap::Butt,
        kurbo::Cap::Round => LineCap::Round,
        kurbo::Cap::Square => LineCap::Square,
    };
    st.join = match p.line_join {
        kurbo::Join::Miter => LineJoin::Miter,
        kurbo::Join::Round => LineJoin::Round,
        kurbo::Join::Bevel => LineJoin::Bevel,
    };
    st.miter_limit = p.miter_limit as f64;
    let dash = Dash { pattern: p.dash_array.iter().map(|v| *v as f64 * scale).collect(), offset: p.dash_offset as f64 * scale, align_corners: false };
    // An invalid dash array (a negative value, or all zeros) strokes solid.
    st.dash = dash.is_dashed().then_some(dash);
    st
}

pub(crate) fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// The paints of `nodes`' leaves, in order: what tells two readings of a pattern cell apart.
fn paints(nodes: &[Arc<Node>]) -> Vec<Paint> {
    let mut out = vec![];
    for n in nodes {
        n.walk(&mut |c| out.extend(c.appearance.items.iter().map(|i| i.paint().clone())));
    }
    out
}

/// Visual bounds of `nodes`; `None` when one has none.
fn bounds(nodes: &[Arc<Node>]) -> Option<Rect> {
    nodes.iter().try_fold(None, |acc, n| n.visual_bounds().map(|b| Some(acc.map_or(b, |a: Rect| a.union(b)))))?
}

impl<'p> Builder<'p> {
    fn new(next: u64, colors: Colors<'p>, text_as: TextAs, route: bool) -> Self {
        Self {
            colors,
            next,
            stack: vec![],
            root_groups: vec![],
            blend: BlendMode::Normal,
            page: Rect::ZERO,
            glyphs: None,
            text: None,
            text_as,
            images: HashMap::new(),
            image_keys: HashMap::new(),
            cmyk_keys: HashSet::new(),
            opaque: HashSet::new(),
            warnings: vec![],
            fonts: HashMap::new(),
            font_names: HashMap::new(),
            families: None,
            missing_fonts: vec![],
            cid_text: HashMap::new(),
            font_chars: HashMap::new(),
            matched: HashMap::new(),
            identified: HashMap::new(),
            versions: HashMap::new(),
            mask: None,
            masked: vec![],
            masks: HashMap::new(),
            patterns: vec![],
            pattern_keys: HashMap::new(),
            taken: vec![],
            scan: Scan::default(),
            tag_at: 0,
            group_at: 0,
            paint_at: 0,
            aligned: true,
            route,
            unsure: false,
            marked: vec![],
            pending: false,
            nested: 0,
            white_images: false,
            last_fill: None,
            lines: HashMap::new(),
        }
    }

    fn id(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    /// A deep copy of `node` with fresh ids.
    fn reid(&mut self, node: &Node) -> Node {
        let mut n = node.clone();
        n.id = self.id();
        if let Some(ch) = n.children_mut() {
            let old = std::mem::take(ch);
            *ch = old.iter().map(|c| Arc::new(self.reid(c))).collect();
        }
        n
    }

    /// `m` with fresh ids for its art: a mask read once can mask several objects.
    fn fresh_mask(&mut self, mut m: OpacityMask) -> OpacityMask {
        m.art = Arc::new(self.reid(&m.art));
        m
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn begin_page(&mut self, scan: Scan) {
        self.stack = vec![Frame::new(FrameKind::Root)];
        self.root_groups.clear();
        self.blend = BlendMode::Normal;
        self.glyphs = None;
        self.text = None;
        self.mask = None;
        self.masked.clear();
        self.marked.clear();
        self.pending = false;
        (self.scan, self.tag_at, self.group_at, self.aligned, self.paint_at) = (scan, 0, 0, true, 0);
        if self.scan.overprints && self.scan.zero_mode {
            self.warn(OVERPRINT_ZERO_MODE);
        }
    }

    /// Whether the path the interpreter draws now (a fill or a stroke, each counted) overprints. It
    /// takes the path's turn in the page's walk.
    fn path_overprint(&mut self) -> bool {
        if self.nested > 0 {
            return false;
        }
        self.paint_at += 1;
        self.scan.overprints && self.scan.paints.get(self.paint_at - 1).copied().unwrap_or(false)
    }

    /// What the interpreter draws now, as the page's walk counts it: the path whose turn comes
    /// next, or the `text` before it. `None` in a nested reading, which the walk doesn't follow.
    fn walked(&self, text: bool) -> Option<Walked> {
        (self.nested == 0).then_some(if text { Walked::Text(self.paint_at) } else { Walked::Path(self.paint_at) })
    }

    /// Whether the glyph the interpreter draws now (its fill, or its `stroke`) overprints.
    fn glyph_overprint(&mut self, stroke: bool) -> bool {
        if self.nested > 0 || !self.scan.overprints {
            return false;
        }
        match self.scan.texts.get(self.paint_at) {
            Some(TextOverprint::All { fill, stroke: s }) => *if stroke { s } else { fill },
            Some(TextOverprint::Mixed) => {
                self.warn("some text is set to overprint in a way that can't be read: it knocks out");
                false
            }
            _ => false,
        }
    }

    /// The page's art, split by optional content group (in the order they first paint).
    fn end_page(&mut self) -> Vec<(Option<usize>, Vec<Arc<Node>>)> {
        let mut children = self.close_frames();
        // The interpreter drew another number of paths than the file was read to paint: which
        // of them overprint can't be trusted.
        if self.scan.overprints && self.paint_at != self.scan.paints.len() {
            children.iter_mut().for_each(vectorcraft_doc::overprint::clear_overprints);
            self.warn(OVERPRINT_UNREAD);
        }
        let groups = std::mem::take(&mut self.root_groups);
        let mut parts: Vec<(Option<usize>, Vec<Arc<Node>>)> = vec![];
        // Thousands of groups: find each one's part by index.
        let mut at: HashMap<Option<usize>, usize> = HashMap::new();
        for (n, g) in children.into_iter().zip(groups.into_iter().chain(std::iter::repeat(None))) {
            let i = *at.entry(g).or_insert_with(|| {
                parts.push((g, vec![]));
                parts.len() - 1
            });
            if let Some((_, v)) = parts.get_mut(i) {
                v.push(n);
            }
        }
        parts
    }

    /// Finish the open runs and frames → the root frame's children.
    fn close_frames(&mut self) -> Vec<Arc<Node>> {
        self.flush();
        while self.stack.len() > 1 {
            self.pop_frame();
        }
        self.stack.pop().map(|f| f.children).unwrap_or_default()
    }

    /// The group art drawn now belongs to (the innermost marked as one).
    fn current_group(&self) -> Option<usize> {
        self.marked.iter().rev().find_map(|g| *g)
    }

    /// Add `n` to the open frame; `group`: the optional content group it was drawn in, if it
    /// knows one (else the current one).
    fn emit(&mut self, n: Arc<Node>, group: Option<usize>) {
        let root = self.stack.len() == 1;
        let current = self.current_group();
        let Some(f) = self.stack.last_mut() else { return };
        f.children.push(n);
        // A frame's art goes where its first art was drawn (a clip can end in a later group).
        if root {
            self.root_groups.push(group.or(current));
        } else {
            f.child_groups.push(group.or(current));
            if f.group.is_none() {
                f.group = group.or(current);
            }
        }
    }

    fn push_node(&mut self, mut n: Node) {
        if self.blend != BlendMode::Normal && !n.is_container() {
            n.blend = self.blend;
        }
        if self.mask.is_some() {
            self.masked.push(Arc::new(n));
        } else {
            self.emit(Arc::new(n), None);
        }
    }

    /// Finish the open glyph runs and the art drawn through the current soft mask.
    fn flush(&mut self) {
        self.flush_glyphs();
        self.flush_text();
        self.flush_masked();
    }

    /// The art drawn through the graphics state's soft mask so far, as one masked object.
    fn flush_masked(&mut self) {
        if self.masked.is_empty() {
            return;
        }
        let art = std::mem::take(&mut self.masked);
        let Some(spec) = self.mask.clone() else {
            art.into_iter().for_each(|n| self.emit(n, None));
            return;
        };
        match spec.as_ref() {
            // Each object is drawn through the mask on its own.
            MaskSpec::Opacity(a) => {
                for mut n in art {
                    Arc::make_mut(&mut n).opacity *= a;
                    self.emit(n, None);
                }
            }
            MaskSpec::Mask(m) => {
                let m = self.fresh_mask(m.clone());
                let node = match <[Arc<Node>; 1]>::try_from(art) {
                    Ok([one]) if one.extras().mask.is_none() => {
                        fold_stop_opacity(&one, &m).unwrap_or_else(|| Arc::unwrap_or_clone(one).with_mask(Some(Box::new(m))))
                    }
                    Ok([one]) => {
                        let mut g = Node::group(self.id(), vec![one]);
                        g.extras_mut().mask = Some(Box::new(m));
                        g
                    }
                    Err(all) => {
                        let mut g = Node::group(self.id(), all);
                        g.extras_mut().mask = Some(Box::new(m));
                        g
                    }
                };
                self.emit(Arc::new(node), None);
            }
        }
    }

    /// A clip path node.
    fn clip_node(&mut self, path: &BezPath, rule: FillRule) -> Node {
        let mut clip = Node::new(self.id(), NodeKind::Path { path: PathData::from_bezpath(path), rule, live: None, clipping: true, guide: false });
        clip.name = Some("<Clipping Path>".into());
        clip
    }

    fn pop_frame(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(f) = self.stack.pop() else { return };
        let group = f.group;
        let node = match f.kind {
            FrameKind::Root => None,
            FrameKind::Skip => {
                grouped(f.children, f.child_groups, group).for_each(|(c, g)| self.emit(c, g));
                None
            }
            FrameKind::Clip(clip) => {
                let noop = clip.path_data().map(|p| p.to_bezpath()).filter(is_rectangle).map(|p| p.bounding_box().inflate(0.01, 0.01));
                if f.children.is_empty() {
                    None
                } else if noop.is_some_and(|r| bounds(&f.children).is_some_and(|b| contains(r, b))) {
                    // A rectangle around all of its art (a form's box) clips nothing.
                    grouped(f.children, f.child_groups, group).for_each(|(c, g)| self.emit(c, g));
                    None
                } else if let Some(shape) = gradient_shape(&clip, &f.children) {
                    // One path, so tools such as the Shape Builder see the shape, not its box (#1304).
                    Some(shape)
                } else {
                    let mut ch = vec![Arc::new(*clip)];
                    ch.extend(f.children);
                    Some(Node::new(self.id(), NodeKind::Group { children: ch, clip: true }))
                }
            }
            FrameKind::Group { opacity, blend, mask, isolate, knockout } => {
                let children = f.children;
                // A constant alpha mask is opacity (of a group that isn't isolated).
                let (opacity, mask) = match mask.as_deref() {
                    Some(MaskSpec::Opacity(a)) => (opacity * a, None),
                    Some(MaskSpec::Mask(m)) => (opacity, Some(m.clone())),
                    None => (opacity, None),
                };
                // Isolation only shows with blending inside; knockout with several objects.
                let isolate = isolate && Node::children_blend(&children, &mut |c| c.blends_through());
                let knockout = knockout && children.len() > 1;
                let plain = opacity >= 0.999 && blend == BlendMode::Normal && mask.is_none() && !isolate && !knockout;
                if children.is_empty() {
                    None
                } else if plain {
                    grouped(children, f.child_groups, group).for_each(|(c, g)| self.emit(c, g));
                    None
                } else if let [only] = children.as_slice()
                    && !only.is_container()
                    && !isolate
                    && !knockout
                    && (mask.is_none() || only.extras().mask.is_none())
                {
                    // A single object in a group: fold opacity, blend and mask into the object.
                    let mut only = (**only).clone();
                    only.opacity *= opacity;
                    if blend != BlendMode::Normal {
                        only.blend = blend;
                    }
                    if let Some(m) = mask {
                        let m = self.fresh_mask(m);
                        only = fold_stop_opacity(&only, &m).unwrap_or_else(|| only.with_mask(Some(Box::new(m))));
                    }
                    Some(only)
                } else {
                    let mut g = Node::group(self.id(), children);
                    (g.opacity, g.blend, g.isolate) = (opacity, blend, isolate);
                    if knockout {
                        g.knockout = Knockout::On;
                    }
                    g.extras_mut().mask = mask.map(|m| Box::new(self.fresh_mask(m)));
                    Some(g)
                }
            }
        };
        if let Some(n) = node {
            self.emit(Arc::new(n), group);
        }
    }

    fn flush_glyphs(&mut self) {
        let Some(run) = self.glyphs.take() else { return };
        if run.path.elements().is_empty() {
            return;
        }
        let mut ap = Appearance::default();
        match &run.stroke {
            None => {
                let mut f = FillLayer::new(run.paint);
                (f.opacity, f.overprint) = (run.opacity, run.overprint);
                ap.items.push(AppearanceItem::Fill(f));
            }
            Some(p) => {
                let mut st = stroke_layer(run.paint, run.opacity, p, run.scale);
                st.overprint = run.overprint;
                ap.items.push(AppearanceItem::Stroke(st));
            }
        }
        let id = self.id();
        let mut n = Node::path(id, PathData::from_bezpath(&run.path), ap);
        n.name = Some(TEXT_OUTLINES.into());
        self.push_node(n);
    }

    fn flush_text(&mut self) {
        let Some(mut line) = self.text.take() else { return };
        let held = std::mem::take(&mut line.held);
        if let Some((t, opacity, facts)) = line.finish() {
            let mut n = Node::new(self.id(), NodeKind::Text(Box::new(t)));
            n.opacity = opacity;
            self.lines.insert(n.id, facts);
            self.push_node(n);
        }
        // The strokes of its glyphs' outlines, right after it ([`import_lines`] makes them its stroke).
        for n in held {
            self.push_node(n);
        }
    }

    /// A stroke of the outline of the glyph just drawn, in the glyph's own paint, while every glyph
    /// before it on the open line was stroked so too, is held with the line instead of ending it
    /// (an editor writes type on a path glyph by glyph, each followed by the stroke added to the
    /// type). `false`: it isn't one. `walked`: the path in the page's walk.
    fn hold_glyph_stroke(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &craft_pdf::interpret::Paint<'_>,
        draw_mode: &PathDrawMode,
        walked: Option<Walked>,
    ) -> bool {
        let PathDrawMode::Stroke(props) = draw_mode else { return false };
        if self.mask.is_some() || self.blend != BlendMode::Normal || matches!(paint, craft_pdf::interpret::Paint::Pattern(_)) {
            return false;
        }
        let (p, opacity) = self.paint(paint, true, walked);
        let Some(line) = &self.text else { return false };
        let Some(ink) = line.ink.last() else { return false };
        if line.held.len() + 1 != line.ink.len() || opacity < 0.999 || !line.last_fill().is_some_and(|f| *f == p) {
            return false;
        }
        let mut bp = path.clone();
        bp.apply_affine(transform);
        let b = bp.bounding_box();
        let tol = 0.01 * (ink.width() + ink.height()) + 0.01;
        if ![(b.x0, ink.x0), (b.y0, ink.y0), (b.x1, ink.x1), (b.y1, ink.y1)].iter().all(|(a, c)| (a - c).abs() <= tol) {
            return false;
        }
        let mut st = stroke_layer(p, opacity, props, mean_scale(transform));
        // Every painted path takes its turn in the scan's overprint list, a held one too.
        st.overprint = self.path_overprint();
        let n = Node::path(self.id(), PathData::from_bezpath(&bp), Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
        if let Some(line) = &mut self.text {
            line.held.push(n);
        }
        true
    }

    /// Convert a hayro paint to a VectorCraft paint and opacity (`stroke`: for a stroke; `walked`:
    /// what it paints in the page's walk, which names its ink).
    fn paint(&mut self, p: &craft_pdf::interpret::Paint<'_>, stroke: bool, walked: Option<Walked>) -> (Paint, f32) {
        let pattern = matches!(p, craft_pdf::interpret::Paint::Pattern(_));
        let inks = walked.and_then(|w| self.scan.ink(w, stroke, pattern));
        match p {
            craft_pdf::interpret::Paint::Color(c) => {
                let [r, g, b, a] = c.to_rgba().components();
                let paint = self.colors.solid(c, inks).map_or_else(|| Paint::solid(Color::rgb(round3(r), round3(g), round3(b))), Native::paint);
                (paint, a)
            }
            craft_pdf::interpret::Paint::Pattern(pat) => match pat.as_ref() {
                Pattern::Shading(sp) => match shading_gradient(sp, &mut self.colors, inks) {
                    Some((g, _)) => (Paint::Gradient(Box::new(g)), sp.opacity),
                    None => {
                        self.warn("function-based shadings (and very large meshes) are imported as a flat colour");
                        (Paint::solid(Color::rgb(0.5, 0.5, 0.5)), sp.opacity)
                    }
                },
                Pattern::Tiling(t) => match self.tiling(t, stroke) {
                    Some(paint) => (paint, 1.0),
                    None => {
                        self.warn("tiling patterns nested too deeply are imported as a flat grey");
                        (Paint::solid(Color::rgb(0.5, 0.5, 0.5)), 1.0)
                    }
                },
            },
        }
    }

    /// Read a soft mask, pattern cell or type 3 glyph (`read`) on its own: its art, in the
    /// coordinates `page` has. `None` when nested too deeply.
    fn nested(&mut self, page: Rect, read: impl FnOnce(&mut Self)) -> Option<Vec<Arc<Node>>> {
        if self.nested >= MAX_NESTED {
            return None;
        }
        self.flush();
        let saved = Saved {
            stack: std::mem::replace(&mut self.stack, vec![Frame::new(FrameKind::Root)]),
            root_groups: std::mem::take(&mut self.root_groups),
            blend: std::mem::replace(&mut self.blend, BlendMode::Normal),
            mask: self.mask.take(),
            page: std::mem::replace(&mut self.page, page),
            pending: std::mem::take(&mut self.pending),
        };
        self.nested += 1;
        read(self);
        let art = self.close_frames();
        self.nested -= 1;
        (self.stack, self.root_groups, self.blend, self.mask, self.page, self.pending) =
            (saved.stack, saved.root_groups, saved.blend, saved.mask, saved.page, saved.pending);
        Some(art)
    }

    /// What soft mask `m` does (read once per mask and placement).
    fn soft_mask(&mut self, m: &SoftMask<'_>) -> Arc<MaskSpec> {
        let key = m.cache_key();
        if let Some(s) = self.masks.get(&key) {
            return s.clone();
        }
        let page = self.page;
        let saved = std::mem::replace(&mut self.white_images, m.mask_type() == MaskType::Alpha);
        let art = self.nested(page, |b| m.interpret(b));
        self.white_images = saved;
        let spec = match art {
            Some(art) => {
                let [r, g, b, _] = m.background_color().to_rgba().components();
                let backdrop = luminance(&Color::rgb(r, g, b));
                let inverted = m.transfer_function().is_some_and(|tf| {
                    let mut ends = [0, 255];
                    tf.apply_to(&mut ends);
                    ends[0] > ends[1]
                });
                let id = self.id();
                mask_spec(art, m.mask_type() == MaskType::Alpha, backdrop, inverted, page, |all| Node::group(id, all))
            }
            None => {
                self.warn("soft masks nested too deeply were left out (content drawn unmasked)");
                MaskSpec::Opacity(1.0)
            }
        };
        let spec = Arc::new(spec);
        self.masks.insert(key, spec.clone());
        spec
    }

    /// Tiling pattern `t` as a pattern swatch's paint (the swatch made the first time).
    fn tiling(&mut self, t: &TilingPattern<'_>, stroke: bool) -> Option<Paint> {
        let key = (t.cache_key(), stroke);
        let tile = Rect::from_origin_size(t.bbox.origin(), (t.x_step.abs().max(1e-3) as f64, t.y_step.abs().max(1e-3) as f64));
        let xf = t.matrix * Affine::translate(tile.origin().to_vec2());
        // An uncoloured pattern draws in the current colour: read it again unless it always
        // drew the same.
        if let Some((n, seen)) = self.pattern_keys.get(&key)
            && *n >= PATTERN_REUSE
            && let [(_, name)] = seen.as_slice()
        {
            return Some(Paint::Pattern { pattern: name.clone(), xf });
        }
        // A swatch is shared with uses outside an alpha mask, and pattern paints keep their
        // colours there: its images are read as they are.
        let white = std::mem::take(&mut self.white_images);
        let art = self.nested(Rect::ZERO, |b| {
            // A cell whose content can't be decoded is empty.
            let _ = t.interpret(b, Affine::IDENTITY, stroke);
        });
        self.white_images = white;
        let art = art?;
        let sig = paints(&art);
        let entry = self.pattern_keys.entry(key).or_default();
        entry.0 += 1;
        let name = match entry.1.iter().find(|(s, _)| *s == sig) {
            Some((_, name)) => name.clone(),
            None => {
                let name = (self.patterns.len() + 1..).map(|i| format!("Pattern {i}")).find(|n| !self.taken.contains(n)).unwrap_or_default();
                self.taken.push(name.clone());
                let mut def = PatternDef::new(&name, art);
                def.tile = tile;
                self.patterns.push(def);
                if let Some(e) = self.pattern_keys.get_mut(&key) {
                    e.1.push((sig, name.clone()));
                }
                name
            }
        };
        Some(Paint::Pattern { pattern: name, xf })
    }

    fn add_image(&mut self, key: u128, make: impl FnOnce() -> Option<(ImageBlob, u32, u32)>, xf: Affine) {
        self.add_image_masked(key, make, xf, None);
    }

    /// An image's alpha (its soft mask, stencil mask or colour key, at its own resolution) as a
    /// greyscale image over the image's `w` × `h` pixels (`transform`): its opacity mask. `None`
    /// when it masks nothing.
    fn alpha_mask(&mut self, key: u128, r: &craft_pdf::interpret::RasterImage<'_>, w: u32, h: u32, transform: Affine) -> Option<OpacityMask> {
        let mkey = key ^ 0x5_3a5c;
        if self.opaque.contains(&mkey) {
            return None;
        }
        let (k, mw, mh) = if let Some(v) = self.image_keys.get(&mkey).cloned() {
            v
        } else {
            let mut alpha = None;
            r.with_rgba(|_, a| alpha = a, None);
            let Some(a) = alpha.filter(|a| a.data.iter().any(|v| *v < 255)) else {
                self.opaque.insert(mkey);
                return None;
            };
            let mut png = Vec::new();
            image::GrayImage::from_raw(a.width, a.height, a.data)?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            let blob = ImageBlob::new("image/png", png);
            let k = blob.content_key();
            self.images.insert(k.clone(), blob);
            self.image_keys.insert(mkey, (k.clone(), a.width, a.height));
            (k, a.width, a.height)
        };
        let xf = transform * Affine::scale_non_uniform(f64::from(w) / f64::from(mw.max(1)), f64::from(h) / f64::from(mh.max(1)));
        let art = Node::new(
            self.id(),
            NodeKind::Image(Box::new(ImageObject { key: k, width: mw, height: mh, xf, link: None, placement: Default::default() })),
        );
        Some(OpacityMask::new(art, true))
    }

    fn add_image_masked(&mut self, key: u128, make: impl FnOnce() -> Option<(ImageBlob, u32, u32)>, xf: Affine, mask: Option<OpacityMask>) {
        let (k, w, h) = if let Some(v) = self.image_keys.get(&key).cloned() {
            v
        } else {
            let Some((blob, w, h)) = make() else {
                self.warn("an image could not be decoded and was skipped");
                return;
            };
            // Content keys: placing this page into another document merges its images safely.
            let k = blob.content_key();
            self.images.insert(k.clone(), blob);
            self.image_keys.insert(key, (k.clone(), w, h));
            (k, w, h)
        };
        let id = self.id();
        let mut n =
            Node::new(id, NodeKind::Image(Box::new(ImageObject { key: k, width: w, height: h, xf, link: None, placement: Default::default() })));
        n.extras_mut().mask = mask.map(Box::new);
        self.push_node(n);
    }

    /// Mesh shadings filling `region` (document space): gradient meshes, clipped to the region
    /// when they reach past it.
    fn draw_meshes(&mut self, meshes: Vec<vectorcraft_doc::live::GradientMesh>, region: &BezPath, opacity: f32) {
        let nodes: Vec<Arc<Node>> = meshes.into_iter().map(|m| Arc::new(Node::new(self.id(), NodeKind::Mesh(m)))).collect();
        let Some(art) = bounds(&nodes) else { return };
        let mut content = match <[Arc<Node>; 1]>::try_from(nodes) {
            Ok([one]) => Arc::unwrap_or_clone(one),
            Err(all) => Node::group(self.id(), all),
        };
        content.opacity *= opacity;
        if !contains(region.bounding_box().inflate(0.5, 0.5), art) {
            let clip = self.clip_node(region, FillRule::NonZero);
            content = clipped(clip, content, self.id());
        }
        self.push_node(content);
    }

    /// The family and style of font `key` (drawing glyph `o`): the installed face of its PostScript
    /// name, else the available family its name reads as; whether it is available.
    /// The character of glyph `glyph` of font `key` that no mapping names, found by its outline in
    /// the installed face of the font's name ([`identify_glyph`]); cached per glyph.
    fn identified_char(&mut self, key: u128, glyph: u32, o: &craft_pdf::interpret::font::OutlineGlyph) -> Option<char> {
        if let Some(c) = self.identified.get(&(key, glyph)) {
            return *c;
        }
        let c = self.font_name(key, o).face.and_then(|f| identify_glyph(&f, &o.outline()));
        self.identified.insert((key, glyph), c);
        c
    }

    fn font_name(&mut self, key: u128, o: &craft_pdf::interpret::font::OutlineGlyph) -> FontInfo {
        if let Some(n) = self.font_names.get(&key) {
            return n.clone();
        }
        let data = o.font_data();
        let name = self.fonts.get(&key).map(|(n, _)| n.clone()).or_else(|| data.as_ref().and_then(|d| d.postscript_name.clone()));
        let (weight, italic) = data.as_ref().map_or((None, false), |d| (d.weight, d.is_italic));
        // A subset's six-letter tag.
        let ps = name.as_deref().map(|n| match n.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest.to_string(),
            _ => n.to_string(),
        });
        let installed = ps.as_deref().and_then(|ps| vectorcraft_text::FontDb::global().find_postscript(ps));
        let n = match (installed, name) {
            (Some(face), _) => FontInfo { family: face.family.clone(), style: face.style.clone(), face: Some(face) },
            (None, Some(name)) => {
                let families = self.families.get_or_insert_with(Families::available);
                let (family, style, found) = families.resolve(&name, weight, italic);
                if !found && !family.is_empty() && !self.missing_fonts.contains(&family) {
                    self.missing_fonts.push(family.clone());
                }
                FontInfo { family, style, face: None }
            }
            (None, None) => {
                let d = vectorcraft_doc::CharStyle::default();
                FontInfo { family: d.font_family, style: d.font_style, face: None }
            }
        };
        self.font_names.insert(key, n.clone());
        n
    }

    /// Glyph `o` as type: added to the line being gathered (or starting one). `false`: it keeps
    /// its outline (no Unicode text, mirrored, or not type mode).
    fn type_glyph(
        &mut self,
        o: &craft_pdf::interpret::font::OutlineGlyph,
        m: Affine,
        scale: f64,
        paint: &craft_pdf::interpret::Paint<'_>,
        stroke: Option<&StrokeProps>,
        overprint: bool,
    ) -> bool {
        if self.text_as != TextAs::Text || self.nested > 0 {
            return false;
        }
        let key = o.font_cache_key();
        let glyph = o.glyph_id().to_u32();
        let (unicode, from_cid) = match o.as_unicode().filter(|u| !replacement(u)) {
            Some(u) => (Some(u), false),
            // No mapping, or one to U+FFFD (#708): the font program may still name the character.
            None => {
                let cid = self.cid_text.entry(key).or_insert_with(|| o.font_data().and_then(|d| CidText::of(d.data.as_ref().as_ref())).map(Arc::new));
                match cid.as_ref().and_then(|c| c.unicode(glyph)) {
                    Some(u) => (Some(u), true),
                    None => {
                        let chars = self
                            .font_chars
                            .entry(key)
                            .or_insert_with(|| o.font_data().and_then(|d| font_chars(d.data.as_ref().as_ref())).map(Arc::new));
                        let named = chars.as_ref().and_then(|m| m.get(&glyph)).copied();
                        // Still unnamed: the installed font may draw the same outline (#811).
                        (named.or_else(|| self.identified_char(key, glyph, o)).map(BfString::Char), false)
                    }
                }
            }
        };
        let text: String = match unicode {
            Some(BfString::Char(c)) => c.to_string(),
            Some(BfString::String(s)) => s,
            None => {
                if o.as_unicode().is_some_and(|u| replacement(&u)) {
                    let name = self.font_name(key, o).family;
                    self.warn(&format!("text in {name} whose characters the file doesn't name (U+FFFD) was kept as outlines"));
                }
                return false;
            }
        };
        // Adobe's CID tables map some ideographs to their look-alike Kangxi radicals.
        let text: String = if from_cid { text.chars().map(unify_radical).collect() } else { text };
        let Some(at) = Placement::of(m) else { return false };
        if text.is_empty() || text.chars().any(|c| c.is_control()) {
            return false;
        }
        let width = o.advance_width().filter(|w| w.is_finite());
        // Set vertically (WMode 1): no horizontal advance; its em box is a full em wide.
        let upright = width.is_some_and(|w| w.abs() <= 1.0);
        let top = m * kurbo::Point::new(f64::from(width.filter(|w| *w > 1.0).unwrap_or(1000.0)) * 0.5, 880.0);
        let mut info = self.font_name(key, o);
        // A font the file doesn't carry is drawn with a stand-in: its glyphs can't tell which
        // style of the installed family it is, so the name decides.
        let embedded = self.fonts.get(&key).is_none_or(|(_, e)| *e);
        if let Some(face) = &info.face
            && embedded
        {
            let decided = match self.matched.get(&key) {
                Some(m) => m.clone(),
                // A glyph that can't be compared (several characters, a vertical form) decides
                // nothing: it takes the face of its name until one that can be compared does.
                None if comparable(&text, upright).is_none() => Some(face.clone()),
                None => {
                    let m = matching_face(face, &text, o, upright);
                    self.matched.insert(key, m.clone());
                    m
                }
            };
            let Some(f) = decided else {
                self.warn("text whose glyphs differ from the installed font of the same name was kept as outlines");
                return false;
            };
            info = FontInfo { family: f.family.clone(), style: f.style.clone(), face: Some(f) };
        }
        // A version of the family and style that the two alone don't resolve to: the type names it.
        // Kept per font once its face is decided (until then each glyph asks).
        let named = |f: &Option<Arc<vectorcraft_text::FontFace>>| {
            let f = f.as_ref()?;
            let default = vectorcraft_text::FontDb::global().face(&f.family, &f.style)?;
            (default.version != f.version).then(|| f.version.clone())
        };
        let version = match self.versions.get(&key) {
            Some(v) => v.clone(),
            None if self.matched.contains_key(&key) => self.versions.entry(key).or_insert_with(|| named(&info.face)).clone(),
            None => named(&info.face),
        };
        let (paint, opacity) = self.paint(paint, stroke.is_some(), self.walked(true));
        let stroke = stroke.map(|p| (paint.clone(), p.line_width as f64 * scale));
        let look = Look {
            font: key,
            family: info.family,
            style: info.style,
            version,
            size: at.size,
            h_scale: at.h_scale,
            overprint_fill: stroke.is_none() && overprint,
            overprint_stroke: stroke.is_some() && overprint,
            fill: stroke.is_none().then_some(paint),
            stroke,
        };
        let advance = width.map_or(at.size * 0.5, |w| w as f64 / 1000.0 * at.size * at.h_scale / 100.0);
        let place = |line: &mut TextLine| {
            if upright { line.push_upright(&look, at, opacity, Upright { top }, &text) } else { line.push(&look, at, opacity, advance, &text) }
        };
        self.flush_glyphs();
        // A stroke over the glyph just filled (fill and stroke rendering).
        if let (Some(s), Some(line)) = (&look.stroke, &mut self.text)
            && look.fill.is_none()
            && line.stroke_last(key, at, s.clone(), overprint)
        {
            return true;
        }
        if !self.text.as_mut().is_some_and(place) {
            self.flush_text();
            let mut line = TextLine::new(at, opacity);
            place(&mut line);
            self.text = Some(line);
        }
        // Its ink: a stroke the file draws as outlines of the glyphs is told by it.
        let mut ink = o.outline();
        if let Some(line) = &mut self.text
            && !ink.elements().is_empty()
        {
            ink.apply_affine(m);
            line.ink.push(ink.bounding_box());
        }
        true
    }
}

fn rgba_png(rgba: Vec<u8>, w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

fn resize_alpha(a: &LumaData, w: u32, h: u32) -> Vec<u8> {
    if a.width == w && a.height == h {
        return a.data.clone();
    }
    match image::GrayImage::from_raw(a.width, a.height, a.data.clone()) {
        Some(g) => image::imageops::resize(&g, w, h, image::imageops::FilterType::Triangle).into_raw(),
        None => vec![255; (w * h) as usize],
    }
}

impl<'a> Device<'a> for Builder<'_> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        let spec = mask.map(|m| self.soft_mask(&m));
        let same = match (&spec, &self.mask) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.flush();
            self.mask = spec;
        }
    }

    fn set_blend_mode(&mut self, blend_mode: craft_pdf::interpret::BlendMode) {
        let b = blend(blend_mode);
        if b != self.blend {
            self.flush_glyphs();
            self.flush_text();
            self.blend = b;
        }
    }

    fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &craft_pdf::interpret::Paint<'a>, draw_mode: &PathDrawMode) {
        self.flush_glyphs();
        let walked = self.walked(false);
        if self.hold_glyph_stroke(path, transform, paint, draw_mode, walked) {
            self.pending = false;
            return;
        }
        self.flush_text();
        self.pending = false;
        let overprint = self.path_overprint();
        let mut bp = path.clone();
        bp.apply_affine(transform);
        if bp.elements().is_empty() {
            return;
        }
        let shading = match paint {
            craft_pdf::interpret::Paint::Pattern(p) => match p.as_ref() {
                Pattern::Shading(sp) => Some(sp),
                Pattern::Tiling(_) => None,
            },
            craft_pdf::interpret::Paint::Color(_) => None,
        };
        // Patch and triangle meshes are gradient meshes.
        if let (PathDrawMode::Fill(_), Some(sp)) = (draw_mode, shading)
            && let Some(meshes) = mesh_shading(sp, &mut self.colors, walked.and_then(|w| self.scan.ink(w, false, true)))
        {
            self.draw_meshes(meshes, &bp, sp.opacity);
            return;
        }
        let pd = PathData::from_bezpath(&bp);
        let stroke = matches!(draw_mode, PathDrawMode::Stroke(_));
        let (paint, opacity) = self.paint(paint, stroke, walked);
        // A gradient that doesn't extend past its ends paints only between them.
        let width = match draw_mode {
            PathDrawMode::Stroke(p) => p.line_width as f64 * mean_scale(transform),
            PathDrawMode::Fill(_) => 0.0,
        };
        let band = shading.and_then(|sp| extend_clip(sp, bp.bounding_box().inflate(width, width)));
        let wrap = |b: &mut Self, n: Node| match &band {
            Some(band) => {
                let clip = b.clip_node(band, FillRule::EvenOdd);
                let id = b.id();
                clipped(clip, n, id)
            }
            None => n,
        };
        match draw_mode {
            PathDrawMode::Fill(rule) => {
                let mut f = FillLayer::new(paint);
                (f.opacity, f.overprint) = (opacity, overprint);
                let mut n = Node::path(self.id(), pd, Appearance { items: vec![AppearanceItem::Fill(f)], ..Default::default() });
                if let NodeKind::Path { rule: r, .. } = &mut n.kind {
                    *r = fill_rule(*rule);
                }
                self.last_fill = Some((n.id, bp));
                let n = wrap(self, n);
                self.push_node(n);
            }
            PathDrawMode::Stroke(props) => {
                let mut st = stroke_layer(paint, opacity, props, mean_scale(transform));
                st.overprint = overprint;
                // Fill-then-stroke of the same path (the `B` operator, or a fill and then a stroke
                // of the same outline) becomes one object: a path with a fill and a live stroke.
                let blend = self.blend;
                let fill = self.last_fill.take();
                let last = match self.mask {
                    Some(_) => self.masked.last_mut(),
                    None => self.stack.last_mut().and_then(|f| f.children.last_mut()),
                };
                if band.is_none()
                    && let Some(last) = last
                    && let Some((id, outline)) = fill
                    && last.id == id
                    && last.blend == blend
                    // Opacity or a mask of its own came from a transparency group the fill was
                    // drawn in and this stroke wasn't.
                    && last.opacity == 1.0
                    && last.extras().mask.is_none()
                    && last.appearance.stroke().is_none()
                    && same_outline(&outline, &bp)
                {
                    let last = Arc::make_mut(last);
                    // The stroke's outline: closing it changes the stroke, not the fill.
                    if let Some(p) = last.path_data_mut() {
                        *p = pd;
                    }
                    last.appearance.items.push(AppearanceItem::Stroke(st));
                    return;
                }
                let n = Node::path(self.id(), pd, Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
                let n = wrap(self, n);
                self.push_node(n);
            }
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.flush();
        // A form's box right after its transparency group: the group's isolate and knockout
        // flags are the next ones read from the file.
        if std::mem::take(&mut self.pending) {
            let flags = self.scan.groups.get(self.group_at).copied();
            self.group_at += 1;
            if let Some((i, k)) = flags
                && let Some(Frame { kind: FrameKind::Group { isolate, knockout, .. }, .. }) = self.stack.last_mut()
            {
                (*isolate, *knockout) = (i, k);
            }
        }
        // Clips that contain the whole page do nothing visible; don't create groups for them. Only
        // a polygon that really covers it counts: a diamond touching the page edges has the page
        // as its bounding box but cuts its corners away (#1542).
        let redundant = clip_path.path.elements().len() <= 6
            && self.page.area() > 0.0
            && vectorcraft_geom::hit::polygon_covers_rect(&clip_path.path, self.page, 0.01);
        let kind = if redundant { FrameKind::Skip } else { FrameKind::Clip(Box::new(self.clip_node(&clip_path.path, fill_rule(clip_path.fill)))) };
        self.stack.push(Frame::new(kind));
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: craft_pdf::interpret::BlendMode) {
        self.flush();
        let mask = mask.map(|m| self.soft_mask(&m));
        self.stack.push(Frame::new(FrameKind::Group { opacity, blend: blend(blend_mode), mask, isolate: false, knockout: false }));
        self.pending = self.nested == 0;
        // Blend mode applies to the group as a whole, not to its children.
        self.blend = BlendMode::Normal;
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &craft_pdf::interpret::Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        self.pending = false;
        let stroke = match draw_mode {
            GlyphDrawMode::Invisible => return,
            GlyphDrawMode::Fill => None,
            GlyphDrawMode::Stroke(p) => Some(p.clone()),
        };
        match glyph {
            Glyph::Outline(o) => {
                let scale = mean_scale(transform);
                let overprint = self.glyph_overprint(stroke.is_some());
                if self.type_glyph(o, transform * glyph_transform, scale, paint, stroke.as_ref(), overprint) {
                    return;
                }
                self.flush_text();
                let mut bp = o.outline();
                bp.apply_affine(transform * glyph_transform);
                let (paint, opacity) = self.paint(paint, stroke.is_some(), self.walked(true));
                let same = self.glyphs.as_ref().is_some_and(|r| {
                    r.paint == paint
                        && r.opacity == opacity
                        && r.overprint == overprint
                        && r.stroke.as_ref().map(|s| s.line_width) == stroke.as_ref().map(|s| s.line_width)
                        && (r.scale - scale).abs() < 1e-9
                });
                if !same {
                    self.flush_glyphs();
                    self.glyphs = Some(GlyphRun { path: BezPath::new(), paint, opacity, stroke, scale, overprint });
                }
                if let Some(r) = &mut self.glyphs {
                    r.path.extend(bp.iter());
                }
                self.warn("text was converted to outlines");
            }
            Glyph::Type3(t3) => {
                if self.nested >= MAX_NESTED {
                    return;
                }
                self.flush();
                self.nested += 1;
                t3.interpret(self, transform, glyph_transform, paint);
                self.nested -= 1;
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        self.flush_glyphs();
        self.flush_text();
        self.pending = false;
        match image {
            Image::Raster(r) => {
                let white = self.white_images;
                // Inside an alpha mask the same image is read again, white (its own cache key).
                let key = craft_pdf::interpret::CacheKey::cache_key(&r) ^ if white { WHITE_KEY } else { 0 };
                let st = r.stream();
                let (w, h) = (r.width(), r.height());
                // CMYK images keep their samples (read once per image).
                let known = !white && self.cmyk_keys.contains(&key);
                let cmyk = if known || white {
                    None
                } else {
                    crate::import_image::cmyk(st, w, h).unwrap_or_else(|why| {
                        self.warn(why);
                        None
                    })
                };
                if known || cmyk.is_some() {
                    self.cmyk_keys.insert(key);
                    // Its mask (read by the interpreter, whatever its kind) becomes an opacity
                    // mask, so the inks stay as they are.
                    let mask = if crate::import_image::has_mask(st) { self.alpha_mask(key, &r, w, h, transform) } else { None };
                    return self.add_image_masked(key, || cmyk.map(|blob| (blob, w, h)), transform, mask);
                }
                // JPEG passthrough for plain DeviceRGB/DeviceGray DCT images.
                let dict = st.dict();
                let filters = st.filters();
                let cs_ok = dict.get::<Name<'_>>(b"ColorSpace").is_some_and(|n| matches!(n.as_ref(), b"DeviceRGB" | b"DeviceGray"));
                let jpeg = !white
                    && filters.len() == 1
                    && matches!(filters[0], craft_pdf::syntax::Filter::DctDecode)
                    && cs_ok
                    && !dict.contains_key(b"SMask")
                    && !dict.contains_key(b"Mask")
                    && !dict.contains_key(b"Decode");
                if jpeg {
                    let bytes = st.raw_data().to_vec();
                    self.add_image(key, || Some((ImageBlob::new("image/jpeg", bytes), w, h)), transform);
                    return;
                }
                let mut decoded: Option<Decoded> = None;
                r.with_rgba(
                    |img, alpha| {
                        let (w, h, sf) = (img.width(), img.height(), img.scale_factors());
                        let a = alpha.map(|a| resize_alpha(&a, w, h)).unwrap_or_else(|| vec![255; (w * h) as usize]);
                        let rgba: Vec<u8> = match img {
                            _ if white => a.into_iter().flat_map(|a| [255, 255, 255, a]).collect(),
                            ImageData::Rgb(d) => d.data.as_chunks::<3>().0.iter().zip(a).flat_map(|(c, a)| [c[0], c[1], c[2], a]).collect(),
                            ImageData::Luma(d) => d.data.iter().zip(a).flat_map(|(g, a)| [*g, *g, *g, a]).collect(),
                        };
                        decoded = Some((rgba, w, h, sf));
                    },
                    None,
                );
                let Some((rgba, w, h, sf)) = decoded else {
                    self.warn("an image could not be decoded and was skipped");
                    return;
                };
                let xf = transform * Affine::scale_non_uniform(sf.0 as f64, sf.1 as f64);
                self.add_image(key, || rgba_png(rgba, w, h).map(|png| (ImageBlob::new("image/png", png), w, h)), xf);
            }
            Image::Stencil(s) => {
                let white = self.white_images;
                let key = craft_pdf::interpret::CacheKey::cache_key(&s) ^ if white { WHITE_KEY } else { 0 };
                let mut decoded: Option<Decoded> = None;
                s.with_stencil(
                    |luma, paint| {
                        let rgb = match paint {
                            _ if white => [255, 255, 255, 255],
                            craft_pdf::interpret::Paint::Color(c) => c.to_rgba().to_rgba8(),
                            _ => [0, 0, 0, 255],
                        };
                        let rgba = luma.data.iter().flat_map(|a| [rgb[0], rgb[1], rgb[2], *a]).collect();
                        decoded = Some((rgba, luma.width, luma.height, luma.scale_factors));
                    },
                    None,
                );
                let Some((rgba, w, h, sf)) = decoded else { return };
                let xf = transform * Affine::scale_non_uniform(sf.0 as f64, sf.1 as f64);
                self.add_image(key ^ 0x5_7e9c, || rgba_png(rgba, w, h).map(|png| (ImageBlob::new("image/png", png), w, h)), xf);
            }
        }
    }

    fn pop_clip_path(&mut self) {
        self.flush();
        self.pending = false;
        self.pop_frame();
    }

    fn pop_transparency_group(&mut self) {
        self.flush();
        self.pending = false;
        self.pop_frame();
    }

    fn begin_marked_content(&mut self, tag: &[u8], _mcid: Option<i32>) {
        if self.nested > 0 {
            return;
        }
        let at = self.tag_at;
        self.tag_at += 1;
        let group = match self.scan.tags.get(at) {
            Some((t, g)) if self.aligned && *t == tag_key(tag) => *g,
            _ => {
                if self.route && self.aligned {
                    self.aligned = false;
                    let went = if self.unsure { "a hidden, non-printing layer of its page" } else { "its page's layer" };
                    self.warn(&format!("some art couldn't be told apart by layer and went to {went}"));
                }
                self.unsure.then_some(UNSORTED)
            }
        }
        .filter(|_| self.route);
        // Only a layer's marked content ends the runs gathered so far: other tags (a span with its
        // actual text, an artifact) leave a line of type whole.
        if group.is_some() {
            self.flush();
        }
        if let Some(g) = group
            && self.stack.len() > 1
            && let Some(f) = self.stack.last_mut()
        {
            f.group.get_or_insert(g);
        }
        self.marked.push(group);
    }

    fn end_marked_content(&mut self) {
        if self.nested > 0 {
            return;
        }
        // The runs gathered within a layer's sequence are its art: finish them before it ends.
        if self.marked.last().copied().flatten().is_some() {
            self.flush();
        }
        self.marked.pop();
    }
}
