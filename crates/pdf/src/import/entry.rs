use super::cid::drawn_paths;
use super::state::Builder;
use super::*;

/// Mixed into an image's cache key when it is read white inside an alpha soft mask.
pub(super) const WHITE_KEY: u128 = 0x7717_e000_a1fa;

/// The name of the paths text imports as.
pub(super) const TEXT_OUTLINES: &str = "<Text Outlines>";

/// A page whose overprint (ISO 32000-1 §8.6.7) is left out rather than given to the wrong objects.
pub(super) const OVERPRINT_UNREAD: &str = "the overprint of a page's objects couldn't be read: they knock out";

/// The document overprints as overprint mode 1 does.
pub(super) const OVERPRINT_ZERO_MODE: &str =
    "overprint in the mode where a process colour's zero tints knock out (/OPM 0) is imported as overprint, where they don't";

/// The deepest soft masks, patterns and type 3 glyphs are read within each other.
pub(super) const MAX_NESTED: u32 = 8;

/// A tiling pattern read this many times with the same art is taken to always draw it.
pub(super) const PATTERN_REUSE: u32 = 16;

/// Stands for the group of art whose group couldn't be told (see [`Builder::unsure`]): it goes
/// to a hidden, non-printing layer of its page.
pub(super) const UNSORTED: usize = usize::MAX;

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

/// Warning: glyphs were kept as outlines rather than read as type.
pub const OUTLINED_TEXT_NOTE: &str = "text was converted to outlines";

/// Warning: text drawn invisibly (text rendering mode 3, as a scanned page's OCR layer is) that
/// can't come in as type (text read as outlines, or a Type 3 font) was left out: as outlines it
/// would draw nothing (#2675).
pub const INVISIBLE_TEXT_NOTE: &str = "invisible text (such as a scanned page's searchable OCR layer) that couldn't be read as type was left out";

/// Warning: glyphs that differ from the installed font of their name were kept as outlines.
pub const UNMATCHED_GLYPHS_NOTE: &str = "text whose glyphs differ from the installed font of the same name was kept as outlines";

/// The end of the warning that glyphs whose characters the file doesn't name were kept as
/// outlines, after "text in" and the font's name.
pub const UNNAMED_CHARACTERS_NOTE: &str = "whose characters the file doesn't name (U+FFFD) was kept as outlines";

/// Is `note` one of the warnings that glyphs were kept as outlines ([`OUTLINED_TEXT_NOTE`],
/// [`UNMATCHED_GLYPHS_NOTE`], [`UNNAMED_CHARACTERS_NOTE`])?
pub fn is_outlines_note(note: &str) -> bool {
    note == OUTLINED_TEXT_NOTE || note == UNMATCHED_GLYPHS_NOTE || note.starts_with("text in ") && note.ends_with(UNNAMED_CHARACTERS_NOTE)
}

/// Is `n` glyphs kept as outlines (a path the import names `<Text Outlines>`)?
pub fn is_glyph_outlines(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { .. }) && n.name.as_deref() == Some(TEXT_OUTLINES)
}

/// Does `doc` hold glyphs kept as outlines ([`is_glyph_outlines`])?
pub fn holds_outlines(doc: &Document) -> bool {
    let mut any = false;
    doc.walk(|n| any |= is_glyph_outlines(n));
    any
}

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
            locked: false,
            show_center_mark: false,
            show_cross_hairs: false,
            show_video_safe_areas: false,
            video_ruler_pixel_aspect_ratio: 1.0,
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
    // Before anything walks the tree by recursion (`fix_next_id` does): nested clips can be deeper
    // than the stack goes.
    let mut depth_notes = vec![];
    doc.limit_depth_noting(&mut depth_notes);
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
    warnings.extend(depth_notes);
    let user_unit = picked.first().and_then(|&n| pages.get(n)).map_or(1.0, |page| f64::from(page.user_unit()));
    Ok(ImportReport { document: doc, warnings, native: crate::editing::editing_in(&pdf), only_text, user_unit })
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
