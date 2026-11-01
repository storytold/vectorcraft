use super::compare::{MAX_DIFFERENCE, PageView, compare, difference};
use super::plan::{made_only, make_slots, plan, readable_all};
use super::slots::{
    ART_LEFT_OUT, Content, MAX_PAIRS, MAX_SLOTS, MAX_TEXT, OUTLINE_REACH, SlotInfo, TEXT_LEFT_OUT, TYPE_LEFT_OUT, collect_slots, extent, few,
    fill_slots, letters, text_objects, words,
};
use super::{Affine, Arc, BTreeSet, Document, Node, NodeId, NodeKind, PathData, Rect, Scaling, Story, Structure, Texts, Unit, Vec2, ai};

/// Which box of the editing copy is the page.
#[derive(Clone, Copy)]
pub(super) enum Page {
    /// An EPS's page is its art's bounding box.
    Art,
    /// A `.ai` is read on its (first and only) artboard.
    Artboard,
}

/// The page (`visible`, the file read as its page) and the place of its area in the layers'
/// document.
pub(super) struct Frame<'a> {
    pub(super) page: &'a Document,
    /// The page's area in `page`.
    pub(super) page_rect: Rect,
    /// The same area in the layers' document.
    pub(super) rect: Rect,
    /// The page holds all the art that prints (an EPS's page is its art's box, unless it was
    /// saved as its artboard): the layers print nothing outside it.
    pub(super) whole: bool,
    /// The PDF page areas and their moves into the editing document.
    pub(super) pages: Vec<(Rect, Affine)>,
}

/// The document of the editing copy `data`, with the text (and strokes round it) of `visible`, the
/// file read as its page, put in its text objects → the document and its notes. Without a page
/// (a `.ai` saved without its PDF part), its type is made from the file's text document alone and
/// nothing is compared. `outlined`: the page's type was read as outlines, so it has none to put in
/// the text objects. `unit`: the size in points of a unit of the page's user space (see
/// [`layered_ai`]).
pub(super) fn build(
    data: &[u8],
    visible: Option<&Document>,
    page: Page,
    outlined: bool,
    trust: bool,
    unit: f64,
) -> Result<(Document, Vec<String>), String> {
    // An EPS (its page is its art) draws a Large Canvas page at full size; a `.ai` keeps its art
    // space, as its PDF part may (#2031).
    build_from(ai::read(data, matches!(page, Page::Art))?, data, visible, page, outlined, trust, unit)
}

/// [`build`] from `structure`, the editing copy `data` already read.
pub(super) fn build_from(
    structure: Structure,
    data: &[u8],
    visible: Option<&Document>,
    page: Page,
    outlined: bool,
    trust: bool,
    unit: f64,
) -> Result<(Document, Vec<String>), String> {
    let Structure {
        mut doc,
        to_doc,
        bbox,
        artboard,
        template,
        hidden_unread,
        unread,
        unread_elsewhere,
        slot_names,
        restyled,
        placed_elsewhere,
        envelopes_drawn,
        warnings,
    } = structure;
    // Editing data in the units of the page's user space ([`in_units`]) is compared with the page
    // scaled down to those units, and the document read from it is scaled up by `unit`.
    let board = artboard.map(|a| to_doc.transform_rect_bbox(a));
    // A page cropped to the art (see below) is in those units as well, when the art's box is and
    // the artboard isn't the page in points.
    let art = bbox.map(|[a, b, c, d]| to_doc.transform_rect_bbox(Rect::new(a, b, c, d)));
    let units = |v: &Document| in_units(board, v, unit) || (!same_size(board, v) && in_units(art, v, unit));
    let small = visible.filter(|v| matches!(page, Page::Artboard) && units(v)).map(|v| {
        let mut v = v.clone();
        scale_document(&mut v, 1.0 / unit);
        v
    });
    let visible = small.as_ref().or(visible);
    let (mut shown, mut hidden) = (vec![], vec![]);
    // The glyphs the page kept as outlines that no text object of it takes in, one by one.
    let mut lone: Vec<Vec<Arc<Node>>> = vec![];
    let mut frame = None;
    if let Some(visible) = visible {
        let art_box = match page {
            Page::Art => bbox.map(|[a, b, c, d]| Rect::new(a, b, c, d)).ok_or("its editing data doesn't say where its art is")?,
            Page::Artboard => artboard.ok_or("its editing data doesn't say where its artboard is")?,
        };
        let page_rect = visible.artboards.first().map(|a| a.rect).ok_or("it has no page")?;
        // The layers' art in the document's terms: a Large Canvas editing copy is written at 1/N
        // scale, which `to_doc` carries, so its art and the page reach the same size (#2031).
        let fits = |r: &Rect| (r.width() - page_rect.width()).abs() <= 0.5 && (r.height() - page_rect.height()).abs() <= 0.5;
        let mut rect = to_doc.transform_rect_bbox(art_box);
        // A PDF saved with some of its artboards has pages for those only, the first of them not
        // always the first artboard: the editing data keeps them all (`first`, the page's).
        let first = shown_artboard(&doc, visible, page, page_rect, fits);
        rect = doc.artboards.get(first).filter(|_| first > 0).map_or(rect, |a| a.rect);
        // A `.ai` whose PDF part's pages are cropped to its art (each page the box of all of it,
        // the editing data's bounding box) rather than to its artboards: the layers are compared
        // with the page on that box, and keep their artboards. Each page then holds all the art
        // again, so every page must be that box.
        let mut cropped = false;
        if matches!(page, Page::Artboard)
            && !fits(&rect)
            && visible.artboards.iter().all(|a| fits(&a.rect))
            && let Some(art) = art.filter(fits)
        {
            (rect, cropped) = (art, true);
        }
        if !fits(&rect) {
            // Both sizes in points as the fields show them, and the editing data's page to the
            // file's as a scale (1:10 is a tenth of its size) when its sides are in proportion and
            // the scale doesn't round to 1:1.
            let n = |v: f64| Unit::Points.number(v);
            let (w, h, pw, ph) = (n(rect.width()), n(rect.height()), n(page_rect.width()), n(page_rect.height()));
            let k = page_rect.width() / rect.width();
            let factor = n(k.max(1.0 / k));
            if k.is_finite() && k > 0.0 && (rect.height() * k - page_rect.height()).abs() <= 0.5 && factor != "1" {
                let scale = if k >= 1.0 { format!("1:{factor}") } else { format!("{factor}:1") };
                return Err(format!("its editing data is for a {w} × {h} pt page, the file's {pw} × {ph} pt page at {scale}"));
            }
            return Err(format!("its editing data is for a {w} × {h} pt page, not the file's {pw} × {ph} pt page"));
        }
        // The page's area in the layers' document, and the page's type moved there.
        let shift = Affine::translate(Vec2::new(rect.x0 - page_rect.x0, rect.y0 - page_rect.y0));
        for l in &visible.layers {
            if let Some(c) = l.children() {
                text_objects(c, l.visible, &mut shown, &mut hidden);
            }
        }
        let taken: BTreeSet<NodeId> = shown.iter().chain(&hidden).flatten().map(|n| n.id).collect();
        visible.walk(|n| {
            if vectorcraft_pdf::is_glyph_outlines(n) && !taken.contains(&n.id) {
                lone.push(vec![Arc::new(n.clone())]);
            }
        });
        // A `.ai` read as its PDF part has a page for each artboard, in a row: the type on each
        // goes where the layers have that artboard.
        let pages: Vec<Rect> = visible.artboards.iter().map(|a| a.rect).collect();
        let boards = page_boards(&doc.artboards, first, &pages);
        let page_moves: Vec<(Rect, Affine)> = pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let move_to_doc =
                    boards.and_then(|boards| boards.get(i)).map(|board| Affine::translate(board.rect.origin() - page.origin())).unwrap_or(shift);
                (*page, move_to_doc)
            })
            .collect();
        // The page with most of an object, the first for what is on none.
        let page_of = |object: &[Arc<Node>]| {
            extent(object).and_then(|r| {
                let inside = |p: &Rect| p.intersect(r).area().max(0.0);
                pages.iter().enumerate().skip(1).filter(|(_, p)| inside(p) > inside(&page_rect)).max_by(|a, b| inside(a.1).total_cmp(&inside(b.1)))
            })
        };
        // Pages cropped to the art are copies of one another: the type is the first page's.
        if cropped {
            for objects in [&mut shown, &mut hidden, &mut lone] {
                objects.retain(|o| page_of(o).is_none());
            }
        }
        for object in shown.iter_mut().chain(hidden.iter_mut()).chain(lone.iter_mut()) {
            let on = page_of(object);
            let shift = match on.and_then(|(i, page)| Some((boards?.get(i)?, page))) {
                Some((board, page)) => Affine::translate(board.rect.origin() - page.origin()),
                None => shift,
            };
            for n in object.iter_mut() {
                Arc::make_mut(n).transform(shift, false);
            }
        }
        // An EPS saved as its artboard has the art outside it left out of its page.
        let artboard_page =
            artboard.is_some_and(|a| [a.x0 - art_box.x0, a.y0 - art_box.y0, a.x1 - art_box.x1, a.y1 - art_box.y1].iter().all(|d| d.abs() <= 1.0));
        let whole = matches!(page, Page::Art) && !artboard_page;
        if whole {
            // Its page is the file's own (its bounding box), as when it comes in as its page: art
            // reaching past the editing data's artboard stays on it (#1102).
            doc.artboards.truncate(1);
            if let Some(a) = doc.artboards.first_mut() {
                a.rect = rect;
            }
            drop_guides_off_boards(&mut doc);
        }
        frame = Some(Frame { page: visible, page_rect, rect, whole, pages: page_moves });
    }
    let mut infos = vec![];
    collect_slots(&doc.layers, true, &mut infos);
    // Each text object's own frame or path, by its story and frame.
    let frames = ai::text_frames(data);
    for info in &mut infos {
        info.art_path = info.story.and_then(|s| frames.get(&(s, info.frame))).map(|p| PathData::from_bezpath(&(to_doc * p.clone())));
    }
    // Type asked for as outlines is the page's: the text objects that show can't be filled.
    if outlined && infos.iter().any(|i| i.shown) {
        return Err("its type is to come in as outlines, which only its page has".into());
    }
    // The page's type inside an unread object's box comes in with the page's art that replaces the
    // object (see [`replace_unread`]), not in a text object.
    let unread_type = take_unread_type(&doc, frame.as_ref(), &unread, &unread_elsewhere, &mut shown);
    // The page's type that needs no text object of the layers: the layers' own legacy text, or
    // placed files that show it.
    let as_outlines = page_type_elsewhere(infos.is_empty(), infos.iter().all(|i| !i.shown), &mut shown, &mut hidden, &doc, &placed_elsewhere)?;
    let texts = Texts::read(data);
    // Each story is read once, however many text objects name it.
    let mut read: std::collections::BTreeMap<u32, Option<Arc<Story>>> = Default::default();
    let stories: Vec<Option<Arc<Story>>> = infos
        .iter()
        .map(|i| {
            let n = i.story?;
            read.entry(n).or_insert_with(|| texts.as_ref().and_then(|t| t.story(n as usize)).map(Arc::new)).clone()
        })
        .collect();
    if stories.iter().flatten().map(|s| s.len()).sum::<usize>() > MAX_TEXT {
        return Err("it has too much text".into());
    }
    let template = template.map(|t| ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0));
    let mut notes = warnings;
    let own = (!outlined).then(|| outside_placed_files(&shown, &doc, &placed_elsewhere));
    let (content, reflow) = slot_content(&infos, &stories, template, to_doc, own.as_deref(), [&shown, &hidden], &mut notes)?;
    // The glyphs left out with the rest of the page's art, and how many text objects take type of
    // the page right next to them (not counted past `MAX_PAIRS` comparisons).
    let compared = lone.len().saturating_mul(shown.iter().chain(&hidden).map(Vec::len).sum());
    let missing: Vec<Rect> = if compared > MAX_PAIRS { vec![] } else { lone.iter().filter_map(|g| extent(g)).collect() };
    let next_to_missing = |n: &Arc<Node>| {
        matches!(n.kind, NodeKind::Text(_))
            && n.visual_bounds().is_some_and(|b| missing.iter().any(|r| !b.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(*r).is_zero_area()))
    };
    let short = content.iter().filter(|c| matches!(c, Content::Page(p) if p.iter().any(next_to_missing))).count();
    let mut empty = 0;
    let mut layers = std::mem::take(&mut doc.layers);
    let mut made = vec![];
    fill_slots(&mut doc, &mut layers, &mut content.into_iter(), &slot_names, &mut empty, &mut made, &mut vec![]);
    doc.layers = layers;
    made.sort();
    let threaded_frames: BTreeSet<NodeId> =
        made.chunk_by(|a, b| a.0 == b.0).filter(|thread| thread.len() > 1).flatten().map(|(_, _, id)| *id).collect();
    if !unread.is_empty() {
        let Some(frame) = frame.as_ref() else { return Err(unread_error(&unread)) };
        let mut copied = BTreeSet::new();
        let kinds = replace_unread(&mut doc, frame, &unread, &unread_elsewhere, &threaded_frames, &mut copied)?;
        // With the text objects filled, the page art chosen can differ from the first try's: the
        // page's type set aside for it, and no other, must be in it.
        let all_copied = |o: &Vec<Arc<Node>>| !o.is_empty() && o.iter().all(|n| copied.contains(&n.id));
        if !unread_type.iter().all(all_copied) || shown.iter().any(all_copied) {
            return Err("the page's type in an unread object's box isn't in its page art".into());
        }
        notes.push(format!("{}: they came in as the page's art", kinds.join(", ")));
    }
    // A story in several frames is threaded type: its frames in order (the first holds the story).
    for thread in made.chunk_by(|a, b| a.0 == b.0) {
        if thread.len() > 1 {
            doc.text_threads.push(thread.iter().map(|(_, _, id)| *id).collect());
        }
    }
    if empty > 0 {
        notes.push(format!("{empty} {}", if frame.is_some() { TEXT_LEFT_OUT } else { TYPE_LEFT_OUT }));
    }
    if short > 0 {
        notes.push(format!("{short} {PAGE_TYPE_WITHOUT_OUTLINES}"));
    }
    if as_outlines > 0 {
        notes.push(format!("{as_outlines} {PAGE_TYPE_AS_OUTLINES}"));
    }
    if !hidden_unread.is_empty() {
        notes.push(format!("{ART_LEFT_OUT} ({}), left out of them", few(hidden_unread)));
    }
    match frame {
        Some(frame) => {
            let mut view = PageView::new(&frame);
            view.restyled(&restyled, frame.rect);
            // The placed files' other placement starts from the layers as read: which text the page
            // doesn't draw is weighed again with the images there, since an image that covered a
            // text object at the first placement may not cover it at the other.
            let other = elsewhere(&doc, &placed_elsewhere);
            let unseen = prune_unseen(&frame, &mut view, &mut doc, reflow);
            let unseen_note = |n: usize| (n > 0).then(|| format!("{n} {UNSEEN_TEXT}"));
            match compare(&frame, &mut view, &doc, reflow) {
                // Placed files at the other point their data gives, where that draws closer to the
                // page: a small image in the wrong place differs too little to fail.
                Ok(note) => {
                    let closer = other.and_then(|mut other| {
                        let unseen = prune_unseen(&frame, &mut view, &mut other, reflow);
                        let closer = view.difference(&other, frame.rect, reflow).any < view.difference(&doc, frame.rect, reflow).any;
                        closer.then(|| compare(&frame, &mut view, &other, reflow).ok()).flatten().map(|note| (other, unseen, note))
                    });
                    match closer {
                        Some((other, unseen, note)) => {
                            doc = other;
                            notes.extend(unseen_note(unseen).into_iter().chain(note));
                        }
                        None => notes.extend(unseen_note(unseen).into_iter().chain(note)),
                    }
                }
                // Placed files at the other point their data gives, where that draws as the page.
                Err(why) => match other.map(|mut other| {
                    let unseen = prune_unseen(&frame, &mut view, &mut other, reflow);
                    (compare(&frame, &mut view, &other, reflow), other, unseen)
                }) {
                    Some((Ok(note), other, unseen)) => {
                        doc = other;
                        notes.extend(unseen_note(unseen).into_iter().chain(note));
                    }
                    _ => {
                        let more = drawn_or_trusted(&frame, &mut view, &mut doc, &envelopes_drawn, &notes, unseen, trust, why, reflow)?;
                        notes.extend(more);
                    }
                },
            }
        }
        None => unmark(&mut doc.layers, true, true, &mut vec![]),
    }
    doc.limit_depth_noting(&mut notes);
    if small.is_some() {
        scale_document(&mut doc, unit);
    }
    Ok((doc, notes))
}

/// A visible page node, with its source order and subtree range.
struct PageArt {
    node: Arc<Node>,
    bounds: Rect,
    page: usize,
    end: usize,
    stateful_ancestor: bool,
}

const ART_MARGIN: f64 = 1.0;

fn inside(art: Rect, bounds: Rect) -> bool {
    art.x0 >= bounds.x0 - ART_MARGIN && art.y0 >= bounds.y0 - ART_MARGIN && art.x1 <= bounds.x1 + ART_MARGIN && art.y1 <= bounds.y1 + ART_MARGIN
}

/// Pick the page holding the most of a box. The first page wins a tie.
fn page_for(bounds: Rect, pages: &[Rect]) -> usize {
    let mut best = (0, 0.0);
    for (i, page) in pages.iter().enumerate() {
        let area = page.intersect(bounds).area().max(0.0);
        if area > best.1 {
            best = (i, area);
        }
    }
    best.0
}

fn collect_live(node: &Node, unread: &BTreeSet<NodeId>, live: &mut Vec<Rect>) {
    if !node.visible || unread.contains(&node.id) {
        return;
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_live(child, unread, live);
        }
    } else if let Some(bounds) = node.visual_bounds() {
        live.push(bounds);
    }
}

fn page_node_has_drawing_state(node: &Node) -> bool {
    node.clips()
        || matches!(node.kind, NodeKind::Path { clipping: true, .. })
        || node.opacity < 1.0
        || node.blend != super::BlendMode::Normal
        || node.isolate
        || node.knockout != vectorcraft_doc::Knockout::Neutral
        || node.extras().mask.is_some()
        || !node.appearance.items.is_empty()
        || !node.appearance.effects.is_empty()
}

fn index_page(node: &Arc<Node>, pages: &[Rect], moves: &[Affine], art: &mut Vec<PageArt>, stateful_ancestor: bool) {
    if !node.visible {
        return;
    }
    let start = art.len();
    let mut indexed = false;
    if let Some(bounds) = node.visual_bounds() {
        let page = page_for(bounds, pages);
        if let Some(shift) = moves.get(page) {
            art.push(PageArt { node: Arc::clone(node), bounds: shift.transform_rect_bbox(bounds), page, end: start + 1, stateful_ancestor });
            indexed = true;
        }
    }
    if let Some(children) = node.children() {
        for child in children {
            index_page(child, pages, moves, art, stateful_ancestor || page_node_has_drawing_state(node));
        }
    }
    let end = art.len();
    if indexed && let Some(entry) = art.get_mut(start) {
        entry.end = end;
    }
}

/// Node IDs belong to the importing document, not to the pattern's drawing.
fn same_pattern_art(existing: &vectorcraft_doc::PatternDef, source: &vectorcraft_doc::PatternDef) -> bool {
    fn clear_ids(node: &mut Node) {
        node.id = NodeId(0);
        if let Some(children) = node.children_mut() {
            for child in children {
                clear_ids(Arc::make_mut(child));
            }
        }
        if let Some(mask) = &mut node.extras_mut().mask {
            clear_ids(Arc::make_mut(&mut mask.art));
        }
    }

    let mut a = existing.clone();
    let mut b = source.clone();
    for pattern in [&mut a, &mut b] {
        for node in &mut pattern.art {
            clear_ids(Arc::make_mut(node));
        }
    }
    a == b
}

/// Bring document resources used by page art across before reassigning its node IDs.
fn copy_page_resources(doc: &mut Document, page: &Document, nodes: &[&Node]) -> Result<(), String> {
    fn collect(node: &Node, images: &mut BTreeSet<String>, patterns: &mut BTreeSet<String>, swatches: &mut BTreeSet<String>) {
        if let NodeKind::Image(im) = &node.kind {
            images.insert(im.key.clone());
        }
        for item in &node.appearance.items {
            match item.paint() {
                vectorcraft_color::Paint::Solid { swatch: Some(name), .. } => {
                    swatches.insert(name.clone());
                }
                vectorcraft_color::Paint::Gradient(gradient) => {
                    if let Some(name) = &gradient.swatch {
                        swatches.insert(name.clone());
                    }
                    for stop in &gradient.gradient.stops {
                        if let Some(name) = &stop.swatch {
                            swatches.insert(name.clone());
                        }
                    }
                }
                vectorcraft_color::Paint::Pattern { pattern, .. } => {
                    patterns.insert(pattern.clone());
                }
                _ => {}
            }
        }
        if let Some(mask) = &node.extras().mask {
            collect(&mask.art, images, patterns, swatches);
        }
        for child in node.children().into_iter().flatten() {
            collect(child, images, patterns, swatches);
        }
    }

    let (mut images, mut patterns, mut swatches) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for node in nodes {
        collect(node, &mut images, &mut patterns, &mut swatches);
    }
    let mut copied = BTreeSet::new();
    while let Some(name) = patterns.pop_first() {
        if !copied.insert(name.clone()) {
            continue;
        }
        let Some(source) = page.pattern(&name) else { return Err(format!("page art is missing pattern {name}")) };
        if let Some(existing) = doc.pattern(&name) {
            if !same_pattern_art(existing, source) {
                return Err(format!("page art conflicts with pattern {name}"));
            }
        } else {
            for node in &source.art {
                collect(node, &mut images, &mut patterns, &mut swatches);
            }
            let mut pattern = source.clone();
            pattern.art = pattern.art.iter().map(|node| Arc::new(reid_page_art(doc, node))).collect();
            doc.patterns.push(pattern);
        }
    }
    for key in images {
        let Some(blob) = page.images.get(&key) else { return Err(format!("page art is missing image data for {key}")) };
        if doc.images.get(&key).is_some_and(|existing| existing.bytes != blob.bytes) {
            return Err(format!("page art conflicts with image data for {key}"));
        }
        doc.images.entry(key).or_insert_with(|| blob.clone());
    }
    for name in swatches {
        let Some(source) = page.swatch(&name) else { return Err(format!("page art is missing swatch {name}")) };
        if let Some(existing) = doc.swatch(&name) {
            if existing != source {
                return Err(format!("page art conflicts with swatch {name}"));
            }
        } else {
            doc.swatches.push(source.clone());
        }
    }
    Ok(())
}

/// The PDF importer keeps group transparency on a group, or folds it into its only child.
/// If the editing object's state cannot be located in that art, use the PDF page as a whole.
/// Every selected page object must carry the state, on itself or down a chain of only children.
fn page_carries_state(editing: &Node, page: &[&Node]) -> bool {
    fn carried(node: &Node, matches: &dyn Fn(&Node) -> bool) -> bool {
        matches(node)
            || match node.children().map(Vec::as_slice) {
                Some([only]) => carried(only, matches),
                _ => false,
            }
    }
    let has = |matches: &dyn Fn(&Node) -> bool| !page.is_empty() && page.iter().all(|node| carried(node, matches));
    let opacity = editing.opacity >= 0.999 || has(&|node: &Node| (node.opacity - editing.opacity).abs() < 0.001);
    let blend = editing.blend == super::BlendMode::Normal || has(&|node: &Node| node.blend == editing.blend);
    let isolate = !editing.isolate || has(&|node: &Node| node.isolate);
    let knockout = matches!(editing.knockout, vectorcraft_doc::Knockout::Neutral | vectorcraft_doc::Knockout::Off)
        || has(&|node: &Node| node.knockout == editing.knockout);
    let appearance = !matches!(editing.kind, NodeKind::Group { .. }) || *editing.appearance == vectorcraft_doc::Appearance::default();
    opacity && blend && isolate && knockout && appearance
}

/// The error for unread objects with no page art to use: what isn't read, as the reader names it.
fn unread_error(unread: &[(NodeId, Vec<String>)]) -> String {
    let kinds: BTreeSet<&str> = unread.iter().flat_map(|(_, kinds)| kinds.iter().map(String::as_str)).collect();
    let (ops, other): (Vec<&str>, Vec<&str>) = kinds.into_iter().partition(|k| k.starts_with('`'));
    let mut list: BTreeSet<String> = other.into_iter().map(String::from).collect();
    if !ops.is_empty() {
        list.insert(format!("operators VectorCraft doesn't know ({})", ops.join(", ")));
    }
    let list: Vec<&str> = list.iter().map(String::as_str).collect();
    format!("it has {}, which VectorCraft doesn't read from it yet", ai::join_and(&list))
}

/// The page's text objects in `shown` that the page objects replacing `doc`'s unread objects hold
/// (see [`replace_unread`], tried on a copy of `doc` before its text objects are filled), taken out
/// of `shown`.
fn take_unread_type(
    doc: &Document,
    frame: Option<&Frame<'_>>,
    unread: &[(NodeId, Vec<String>)],
    elsewhere: &[(NodeId, Rect)],
    shown: &mut Vec<Vec<Arc<Node>>>,
) -> Vec<Vec<Arc<Node>>> {
    let Some(frame) = frame.filter(|_| !unread.is_empty() && !shown.is_empty()) else { return vec![] };
    let mut copied = BTreeSet::new();
    if replace_unread(&mut doc.clone(), frame, unread, elsewhere, &BTreeSet::new(), &mut copied).is_err() {
        return vec![];
    }
    let (taken, kept) = std::mem::take(shown).into_iter().partition(|o| !o.is_empty() && o.iter().all(|n| copied.contains(&n.id)));
    *shown = kept;
    taken
}

/// Replace each unread top-level object with painted page objects inside its readable bounds.
/// `copied` gets the ids of the page's nodes copied.
fn replace_unread(
    doc: &mut Document,
    frame: &Frame<'_>,
    unread: &[(NodeId, Vec<String>)],
    elsewhere: &[(NodeId, Rect)],
    threaded_frames: &BTreeSet<NodeId>,
    copied: &mut BTreeSet<NodeId>,
) -> Result<Vec<String>, String> {
    fn holds_threaded_frame(node: &Node, frames: &BTreeSet<NodeId>) -> bool {
        frames.contains(&node.id) || node.children().is_some_and(|children| children.iter().any(|child| holds_threaded_frame(child, frames)))
    }
    fn subtree(node: &Node, ids: &mut BTreeSet<NodeId>) {
        ids.insert(node.id);
        for child in node.children().into_iter().flatten() {
            subtree(child, ids);
        }
    }

    let unread_ids: BTreeSet<NodeId> = unread.iter().map(|(id, _)| *id).collect();
    let mut live = Vec::new();
    for layer in &doc.layers {
        collect_live(layer, &unread_ids, &mut live);
    }
    live.sort_by(|a, b| a.x0.total_cmp(&b.x0));

    let source_pages: Vec<Rect> = frame.pages.iter().map(|(page, _)| *page).collect();
    let moves: Vec<Affine> = frame.pages.iter().map(|(_, shift)| *shift).collect();
    let destination_pages: Vec<Rect> = frame.pages.iter().map(|(page, shift)| shift.transform_rect_bbox(*page)).collect();
    let mut art = Vec::new();
    for layer in &frame.page.layers {
        if layer.visible
            && let Some(children) = layer.children()
        {
            for child in children {
                index_page(child, &source_pages, &moves, &mut art, false);
            }
        }
    }
    let mut by_x: Vec<usize> = (0..art.len()).collect();
    by_x.sort_by(|a, b| art[*a].bounds.x0.total_cmp(&art[*b].bounds.x0));
    let mut claimed = std::collections::BTreeMap::<usize, usize>::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for (id, kinds) in unread {
        let Some((bounds, name, visible, locked, editing)) =
            doc.node(*id).and_then(|n| n.visual_bounds().map(|b| (b, n.name.clone(), n.visible, n.locked, n.clone())))
        else {
            return Err("an unread object has no readable bounds".into());
        };
        if !bounds.area().is_finite() || bounds.area() <= 0.0 {
            return Err("an unread object has no readable bounds".into());
        }
        let mut selected = Vec::new();
        let mut page = 0;
        let mut first_rejection = None;
        'candidate: for candidate in std::iter::once(bounds).chain(elsewhere.iter().filter(|(other, _)| *other == *id).map(|(_, b)| *b)) {
            let live_start = live.partition_point(|b| b.x0 < candidate.x0 - ART_MARGIN);
            let live_end = live.partition_point(|b| b.x0 <= candidate.x1 + ART_MARGIN);
            if live[live_start..live_end].iter().any(|art| inside(*art, candidate)) {
                first_rejection.get_or_insert("an unread object's bounds also contain live art");
                continue;
            }
            // Each page has its own copy of art across artboards: one page's art can't stand in for it.
            if destination_pages.iter().filter(|page| page.intersect(candidate).area() > 0.0).count() > 1 {
                first_rejection.get_or_insert("an unread object lies on more than one artboard");
                continue;
            }
            let candidate_page = page_for(candidate, &destination_pages);
            // An artboard that has no page (a PDF part saved with some of the artboards) has no art
            // on the page for the object's part on it.
            if let Some(page) = destination_pages.get(candidate_page)
                && doc.artboards.iter().map(|a| a.rect.intersect(candidate)).any(|part| part.area() > page.intersect(part).area().max(0.0))
            {
                first_rejection.get_or_insert("an unread object lies on an artboard that has no page");
                continue;
            }
            let start = by_x.partition_point(|i| art[*i].bounds.x0 < candidate.x0 - ART_MARGIN);
            let end = by_x.partition_point(|i| art[*i].bounds.x0 <= candidate.x1 + ART_MARGIN);
            let mut candidates: Vec<usize> =
                by_x[start..end].iter().copied().filter(|i| art[*i].page == candidate_page && inside(art[*i].bounds, candidate)).collect();
            candidates.sort_unstable();
            let mut covered_until = 0;
            let mut candidate_selected = Vec::new();
            for i in candidates {
                if i < covered_until || matches!(art[i].node.kind, NodeKind::Path { clipping: true, .. }) {
                    continue;
                }
                if art[i].stateful_ancestor {
                    first_rejection.get_or_insert("an unread object's page art depends on a clipping or appearance ancestor");
                    continue 'candidate;
                }
                covered_until = art[i].end;
                candidate_selected.push(i);
            }
            if !candidate_selected.is_empty()
                && !page_carries_state(&editing, &candidate_selected.iter().map(|&i| art[i].node.as_ref()).collect::<Vec<_>>())
            {
                first_rejection.get_or_insert("an unread object's appearance is not identifiable in the page art");
                continue;
            }
            if candidate_selected.iter().any(|&i| claimed.range(..art[i].end).next_back().is_some_and(|(_, end)| *end > i)) {
                first_rejection.get_or_insert("one page object matches more than one unread object");
                continue;
            }
            if !candidate_selected.is_empty() {
                selected = candidate_selected;
                page = candidate_page;
                break;
            }
        }
        if selected.is_empty() {
            return Err(first_rejection.unwrap_or("an unread object's page art wasn't found in its bounds").into());
        }
        if holds_threaded_frame(&editing, threaded_frames) {
            return Err("an unread object holds a frame of threaded type".into());
        }
        copy_page_resources(doc, frame.page, &selected.iter().map(|&i| art[i].node.as_ref()).collect::<Vec<_>>())?;
        let mut found = Vec::new();
        for &i in &selected {
            let mut copy = Node::clone(&art[i].node);
            if let Some(shift) = moves.get(page) {
                copy.transform(*shift, false);
            }
            found.push(Arc::new(reid_page_art(doc, &copy)));
            claimed.insert(i, art[i].end);
            subtree(&art[i].node, copied);
        }
        let mut replacement = Node::group(*id, found);
        replacement.name = name;
        replacement.visible = visible;
        replacement.locked = locked;
        let Some(target) = doc.node_mut(*id) else { return Err("an unread object is missing from its layer".into()) };
        *target = replacement;
        for kind in kinds {
            *counts.entry(kind.clone()).or_default() += 1;
        }
    }
    Ok(counts.into_iter().map(|(kind, count)| format!("{kind} ({count})")).collect())
}

/// Copy page drawing IDs, including mask art outside the ordinary child tree.
fn reid_page_art(doc: &mut Document, source: &Node) -> Node {
    let mut copy = doc.reid(source);
    if let Some(mask) = &mut copy.extras_mut().mask {
        mask.art = Arc::new(reid_page_art(doc, &mask.art));
    }
    if let Some(children) = copy.children_mut() {
        for child in children {
            let node = reid_page_art_masks(doc, child);
            *child = Arc::new(node);
        }
    }
    copy
}

fn reid_page_art_masks(doc: &mut Document, source: &Node) -> Node {
    let mut copy = source.clone();
    if let Some(mask) = &mut copy.extras_mut().mask {
        mask.art = Arc::new(reid_page_art(doc, &mask.art));
    }
    if let Some(children) = copy.children_mut() {
        for child in children {
            *child = Arc::new(reid_page_art_masks(doc, child));
        }
    }
    copy
}

/// What goes into each of `infos`' slots, and whether the type is all made from the file's text
/// document. When the text document makes the type of every text object that shows (and the type
/// isn't asked for as outlines: `own` has the page's text objects that show outside placed files),
/// each slot takes its own made type and the page's type is only compared with it: type laid out
/// again, in another font where the file's isn't installed, may differ from the page's. Otherwise
/// see [`plan`], with `pages` (the page's text objects that show and that don't).
fn slot_content(
    infos: &[SlotInfo],
    stories: &[Option<Arc<Story>>],
    template: Option<(f64, f64)>,
    to_doc: Affine,
    own: Option<&[Vec<Arc<Node>>]>,
    pages: [&[Vec<Arc<Node>>]; 2],
    notes: &mut Vec<String>,
) -> Result<(Vec<Content>, bool), String> {
    let made = make_slots(infos, stories, template, to_doc, notes);
    if let Some(own) = own.filter(|_| readable_all(infos, &made)) {
        return Ok((made_only(infos, made, [own, pages[1]], notes), true));
    }
    // The page's type is matched with every text object whose type isn't made, and weighed against
    // where every made one draws: that has its limits.
    let open = made.iter().filter(|m| m.is_none()).count();
    if open > MAX_SLOTS || infos.len().saturating_mul(pages[0].len()) > MAX_PAIRS {
        return Err("it has too many text objects".into());
    }
    Ok((plan(infos, stories, made, template, to_doc, pages, notes), false))
}

/// The layers `doc` don't draw as the page (`why`), `unseen` text objects of them taken out: the
/// notes to add when `doc` with each live envelope `drawn` lists replaced by the drawn art it took
/// the place of (see [`ai::Structure::envelopes_drawn`]) draws as the page, `doc` then being that;
/// else, with `trust`, the notes for `doc` as it is. An error, `why`, otherwise. The envelope note
/// is added unless `notes` has it.
#[allow(clippy::too_many_arguments)]
fn drawn_or_trusted(
    frame: &Frame<'_>,
    view: &mut PageView,
    doc: &mut Document,
    drawn: &[(NodeId, Node)],
    notes: &[String],
    unseen: usize,
    trust: bool,
    why: String,
    reflow: bool,
) -> Result<Vec<String>, String> {
    let unseen_note = |n: usize| (n > 0).then(|| format!("{n} {UNSEEN_TEXT}"));
    let mut put_back = doc.clone();
    let mut any = false;
    for (id, art) in drawn {
        if let Some(n) = put_back.node_mut(*id) {
            *n = art.clone();
            any = true;
        }
    }
    if any {
        let more = prune_unseen(frame, view, &mut put_back, reflow);
        if let Ok(note) = compare(frame, view, &put_back, reflow) {
            *doc = put_back;
            let envelope = (!notes.iter().any(|n| n == ai::ENVELOPE_DRAWN)).then(|| ai::ENVELOPE_DRAWN.to_string());
            return Ok(envelope.into_iter().chain(unseen_note(unseen + more)).chain(note).collect());
        }
    }
    if trust {
        return Ok(unseen_note(unseen).into_iter().chain([format!("{why}: they come from its editing data all the same")]).collect());
    }
    Err(why)
}

/// `doc` with the placed files `placed` lists at their other placement (see
/// [`ai::Structure::placed_elsewhere`]): each image's transform followed by its move. `None` when
/// no listed image survives replacement.
fn elsewhere(doc: &Document, placed: &[(NodeId, Affine)]) -> Option<Document> {
    if placed.is_empty() {
        return None;
    }
    let mut doc = doc.clone();
    let mut moved = false;
    for (id, shift) in placed {
        if let Some(NodeKind::Image(im)) = doc.node_mut(*id).map(|n| &mut n.kind) {
            im.xf *= *shift;
            moved = true;
        }
    }
    moved.then_some(doc)
}

/// The page's type that needs no text object of the layers, taken out of `shown` and `hidden` (the
/// page's text objects that show and that don't): how many of them the layers have as outlines.
/// Type in the legacy format (`To` … `TO`, as files saved for Illustrator 10 and earlier keep it)
/// is read as text objects of the layers' own, with no slot (`no_slots`) for the page's type; the
/// page's type that their words don't hold is outlines on the layers, and the comparison with the
/// page says whether they draw it. Without such text and with no slot that shows (`none_shown`),
/// the page's type must all lie inside placed files, which show it.
fn page_type_elsewhere(
    no_slots: bool,
    none_shown: bool,
    shown: &mut Vec<Vec<Arc<Node>>>,
    hidden: &mut Vec<Vec<Arc<Node>>>,
    doc: &Document,
    placed: &[(NodeId, Affine)],
) -> Result<usize, String> {
    let mut own = vec![];
    own_text(&doc.layers, &mut own);
    let mut as_outlines = 0;
    if no_slots && !own.is_empty() {
        as_outlines = shown.iter().filter(|o| !in_own_text(o, &own)).count();
        shown.clear();
        hidden.clear();
    }
    if none_shown && !shown.is_empty() && own.is_empty() {
        if !in_placed_files(shown, doc, placed) {
            return Err("its page has text, and its layers have no text objects to put it in".into());
        }
        // The page's type is all inside placed files: it is theirs, which their previews show, and
        // needs no text object.
        shown.clear();
    }
    Ok(as_outlines)
}

/// Is the page's text object `object` the layers' own text (`own`, from [`own_text`]): one with its
/// words next to it? The rest of the page's type is what the editing data keeps as outlines (what
/// the legacy format can't hold), and the comparison with the page says whether the layers draw it.
fn in_own_text(object: &[Arc<Node>], own: &[(Rect, String)]) -> bool {
    let Some(r) = extent(object) else { return false };
    let near: String =
        own.iter().filter(|(b, _)| !b.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(r).is_zero_area()).map(|(_, l)| l.as_str()).collect();
    let page_words: Vec<String> = object
        .iter()
        .filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(words(&t.plain_text().to_lowercase())) } else { None })
        .flatten()
        .collect();
    !near.is_empty() && page_words.iter().all(|w| near.contains(w.as_str()))
}

/// The text objects of their own (not slots for the page's type) that show in `nodes`: their
/// bounds and their letters, in lower case.
fn own_text(nodes: &[Arc<Node>], out: &mut Vec<(Rect, String)>) {
    for n in nodes.iter().filter(|n| n.visible) {
        match &n.kind {
            NodeKind::Text(t) => out.extend(n.visual_bounds().map(|b| (b, letters(&t.plain_text().to_lowercase())))),
            NodeKind::Compound { .. } => {}
            _ => {
                if let Some(c) = n.children() {
                    own_text(c, out);
                }
            }
        }
    }
}

/// Has `r` the size of `page`'s first artboard (in points)?
fn same_size(r: Option<Rect>, page: &Document) -> bool {
    let (Some(r), Some(p)) = (r, page.artboards.first().map(|a| a.rect)) else { return false };
    (r.width() - p.width()).abs() <= 0.5 && (r.height() - p.height()).abs() <= 0.5
}

/// Does all of `objects` (the page's text objects) lie inside placed files that show in `doc`? The
/// page draws a placed file, type and all, where the layers show its preview (a linked image, see
/// `ai::Reader::end_place`), at its first placement or at the other one that `placed` lists (see
/// [`elsewhere`]).
fn in_placed_files(objects: &[Vec<Arc<Node>>], doc: &Document, placed: &[(NodeId, Affine)]) -> bool {
    let inside = |d: &Document| {
        let mut files = vec![];
        placed_files(&d.layers, &mut files);
        !files.is_empty()
            && objects
                .iter()
                .all(|o| extent(o).is_some_and(|r| files.iter().any(|f: &Rect| f.inflate(OUTLINE_REACH, OUTLINE_REACH).contains_rect(r))))
    };
    inside(doc) || elsewhere(doc, placed).is_some_and(|other| inside(&other))
}

/// The page's text objects in `objects` that don't lie inside a placed file that shows in `doc`, at
/// either placement (see [`in_placed_files`]): the placed file's preview has the others.
fn outside_placed_files(objects: &[Vec<Arc<Node>>], doc: &Document, placed: &[(NodeId, Affine)]) -> Vec<Vec<Arc<Node>>> {
    let mut files = vec![];
    placed_files(&doc.layers, &mut files);
    if let Some(other) = elsewhere(doc, placed) {
        placed_files(&other.layers, &mut files);
    }
    if files.is_empty() {
        return objects.to_vec();
    }
    let inside = |o: &[Arc<Node>]| extent(o).is_some_and(|r| files.iter().any(|f| f.inflate(OUTLINE_REACH, OUTLINE_REACH).contains_rect(r)));
    objects.iter().filter(|o| !inside(o)).cloned().collect()
}

/// Where the placed files that show in `nodes` are: the linked images the editing data's reader
/// makes of them.
fn placed_files(nodes: &[Arc<Node>], out: &mut Vec<Rect>) {
    for n in nodes.iter().filter(|n| n.visible) {
        match &n.kind {
            NodeKind::Image(im) if im.link.is_some() => out.extend(n.visual_bounds()),
            _ => {
                if let Some(c) = n.children() {
                    placed_files(c, out);
                }
            }
        }
    }
}

/// Does the editing data's first artboard `board` (in the document read from it: art space through
/// `to_doc`) have the size of `page`'s first artboard in units of `unit` points, `unit` being above
/// 1, and not its size in points? `page` is a PDF part read in points from pages of `unit` points a
/// unit (their `UserUnit`).
fn in_units(board: Option<Rect>, page: &Document, unit: f64) -> bool {
    let (Some(board), Some(p)) = (board, page.artboards.first().map(|a| a.rect)) else { return false };
    let fits = |k: f64| (board.width() - p.width() / k).abs() <= 0.5 && (board.height() - p.height() / k).abs() <= 0.5;
    unit.is_finite() && unit > 1.0 && fits(unit) && !fits(1.0)
}

/// The most artboards [`shown_artboard`] weighs, a render each.
const MAX_BOARDS_WEIGHED: usize = 16;

/// Which of the artboards of `doc` (the layers' document) the page `page_rect` of `visible` shows,
/// when `visible` has fewer pages than `doc` has artboards (a PDF saved with a range of them): of
/// those of the page's size (`fits`), the one whose art draws most like the page, the first of
/// them on a tie. Else (and for an EPS, whose page is its art) the first artboard (0).
fn shown_artboard(doc: &Document, visible: &Document, page: Page, page_rect: Rect, fits: impl Fn(&Rect) -> bool) -> usize {
    if matches!(page, Page::Art) || visible.artboards.len() >= doc.artboards.len() {
        return 0;
    }
    let sized: Vec<usize> = doc.artboards.iter().enumerate().filter(|(_, a)| fits(&a.rect)).map(|(i, _)| i).take(MAX_BOARDS_WEIGHED).collect();
    if let [only] = sized.as_slice() {
        return *only;
    }
    let mut best: Option<(usize, f64)> = None;
    for (i, a) in sized.iter().filter_map(|&i| Some((i, doc.artboards.get(i)?))) {
        let d = difference(visible, page_rect, doc, a.rect).any;
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((i, d));
        }
    }
    best.map_or(0, |(i, _)| i)
}

/// The artboards of the layers' document (`boards`) that the pages show, one a page from the
/// first page's (`first`), when they are the pages' sizes: every artboard, or those of a PDF saved
/// with some of them (`first` after the first artboard).
fn page_boards<'a>(boards: &'a [vectorcraft_doc::Artboard], first: usize, pages: &[Rect]) -> Option<&'a [vectorcraft_doc::Artboard]> {
    let same = |(a, p): (&vectorcraft_doc::Artboard, &Rect)| (a.rect.width() - p.width()).abs() <= 0.5 && (a.rect.height() - p.height()).abs() <= 0.5;
    let shown = boards.get(first..first.saturating_add(pages.len()))?;
    ((first > 0 || pages.len() == boards.len()) && shown.iter().zip(pages).all(same)).then_some(shown)
}

/// Drops the guides of artboards `doc` no longer has: an artboard guide goes with its artboard.
fn drop_guides_off_boards(doc: &mut Document) {
    let kept: Vec<u32> = doc.artboards.iter().map(|a| a.id).collect();
    doc.guides.retain(|g| g.artboard.is_none_or(|id| kept.contains(&id)));
}

/// Draw `doc`'s layers, artboards and guides `k` times as large, from the origin, with their
/// strokes, effects and pattern tiles.
fn scale_document(doc: &mut Document, k: f64) {
    let a = Affine::scale(k);
    let scaling = Scaling { strokes: true, effects: Some(vectorcraft_effects::scale_effect), patterns: true, ..Default::default() };
    for layer in &mut doc.layers {
        Arc::make_mut(layer).transform(a, scaling);
    }
    for board in &mut doc.artboards {
        board.rect = a.transform_rect_bbox(board.rect);
    }
    for guide in &mut doc.guides {
        guide.pos *= k;
    }
}

/// Marks (in a node's name, until [`prune_unseen`] has looked) the text objects made from the file's
/// text document.
pub(super) const MADE: &str = "\u{1}made";

/// How many such objects are weighed against the page (each takes a render of the page's size).
const MAX_WEIGHED: usize = 12;

/// The text objects the file keeps that the page doesn't draw.
const UNSEEN_TEXT: &str = "text objects that the file keeps but its page doesn't draw (what is left of type turned to outlines) are left out";

/// The text objects that hold the page's type next to glyphs the page kept as outlines, which are
/// left out.
const PAGE_TYPE_WITHOUT_OUTLINES: &str = "text objects hold their type as the page draws it, without the characters the page has only as outlines";

/// The page's text objects that the layers have as outlines, as the editing data keeps them.
const PAGE_TYPE_AS_OUTLINES: &str = "text objects of the page are outlines on the layers, as the file's editing data keeps them";

/// How much more of the page (0 to 1) the layers may differ without a text object and still look no worse.
const UNSEEN_MARGIN: f64 = 5e-5;

/// Clear the marks in `nodes` and say which marked objects show and print, by id.
fn unmark(nodes: &mut [Arc<Node>], shown: bool, prints: bool, found: &mut Vec<(NodeId, Option<Rect>)>) {
    for n in nodes {
        let shown = shown && n.visible;
        let prints = prints && !matches!(n.kind, NodeKind::Layer { printable: false, .. });
        if let Some(name) = n.name.as_deref().and_then(|m| m.strip_prefix(MADE)) {
            if shown && prints {
                found.push((n.id, n.visual_bounds()));
            }
            let name = (!name.is_empty()).then(|| name.to_string());
            Arc::make_mut(n).name = name;
        } else if !matches!(n.kind, NodeKind::Compound { .. })
            && let Some(c) = Arc::make_mut(n).children_mut()
        {
            unmark(c, shown, prints, found);
        }
    }
}

/// Take object `id` out of `nodes` (`hide`: leave it, hidden); whether it was there.
fn take_out(nodes: &mut Vec<Arc<Node>>, id: NodeId, hide: bool) -> bool {
    if let Some(i) = nodes.iter().position(|n| n.id == id) {
        if hide {
            if let Some(n) = nodes.get_mut(i) {
                Arc::make_mut(n).visible = false;
            }
        } else {
            nodes.remove(i);
        }
        return true;
    }
    nodes.iter_mut().any(|n| !matches!(n.kind, NodeKind::Compound { .. }) && Arc::make_mut(n).children_mut().is_some_and(|c| take_out(c, id, hide)))
}

/// The text objects made from the file's text document that show and print on the page's area,
/// where the page has no type to match them, and that the layers look no worse without, are not
/// drawn by the app that wrote the file (which paints every shown text object; it keeps the type
/// of what was turned to outlines): they are taken out. How many. `reflow`: the type is all laid
/// out again from the text document (see [`compare`]).
pub(super) fn prune_unseen(frame: &Frame<'_>, view: &mut PageView, doc: &mut Document, reflow: bool) -> usize {
    let mut found = vec![];
    unmark(&mut doc.layers, true, true, &mut found);
    let rect = frame.rect;
    if found.is_empty() || found.len() > MAX_WEIGHED {
        return 0;
    }
    let with = view.difference(doc, rect, reflow).weighed(reflow);
    // Taking a text object out changes only what is drawn in its box. When the art would still
    // differ from the page by more than `compare` allows with every candidate gone, the layers
    // aren't used whatever is pruned: don't draw them again for each one (#758).
    let margin = OUTLINE_REACH + 1.0;
    let boxes: f64 = found.iter().filter_map(|(_, b)| *b).map(|b| b.inflate(margin, margin).intersect(rect).area().max(0.0)).sum();
    if with - boxes / rect.area().max(1e-9) > MAX_DIFFERENCE {
        return 0;
    }
    // Weighed by colour, not only lightness: a text object's red stroke over a dark red one is no
    // less there for being about as light (#1102).
    let mut with = view.colour_difference(doc, rect);
    let mut removed = 0;
    for (id, bounds) in found {
        if bounds.is_none_or(|b| b.intersect(rect).is_zero_area()) {
            continue;
        }
        let mut without = doc.clone();
        if !take_out(&mut without.layers, id, true) {
            continue;
        }
        let d = view.colour_difference(&without, rect);
        if d - with <= UNSEEN_MARGIN {
            take_out(&mut doc.layers, id, false);
            with = d;
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod candidate_tests {
    use super::*;

    use vectorcraft_doc::{ImageBlob, ImageObject};

    /// A page image with a different proxy keeps its editing layer and original blob.
    #[test]
    fn page_image_with_different_proxy_keeps_editing_layer() {
        let mut doc = Document::new(100.0, 100.0);
        let placeholder_id = doc.alloc_id();
        let placeholder = Node::path(placeholder_id, vectorcraft_geom::shapes::rectangle(Rect::new(50.0, 50.0, 54.0, 54.0)), Default::default());
        doc.insert(Some(doc.layers[0].id), 0, placeholder).unwrap();

        let mut page = Document::new(100.0, 100.0);
        let mut editing_blob = ImageBlob::new("image/png", vec![1, 2, 3]);
        editing_blob.proxy = Some(Arc::new(vec![4, 5, 6]));
        let page_blob = ImageBlob::new("image/png", vec![1, 2, 3]);
        let key = editing_blob.content_key();
        assert_eq!(key, page_blob.content_key());
        doc.images.insert(key.clone(), editing_blob.clone());
        page.images.insert(key.clone(), page_blob);
        let image = Node::new(
            page.alloc_id(),
            NodeKind::Image(Box::new(ImageObject {
                key: key.clone(),
                width: 4,
                height: 4,
                xf: Affine::translate(Vec2::new(50.0, 50.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        );
        page.insert(Some(page.layers[0].id), 0, image).unwrap();

        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let kinds = replace_unread(
            &mut doc,
            &frame,
            &[(placeholder_id, vec!["placed files".into()])],
            &[(placeholder_id, Rect::new(50.0, 50.0, 54.0, 54.0))],
            &BTreeSet::new(),
            &mut BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(kinds, ["placed files (1)"]);
        let replacement = doc.node(placeholder_id).unwrap();
        let [image] = replacement.children().unwrap().as_slice() else { panic!("{replacement:?}") };
        assert!(matches!(image.kind, NodeKind::Image(_)));
        assert_eq!(doc.images.get(&key), Some(&editing_blob));
    }

    /// An unread object across two artboards isn't replaced by the art of one page only.
    #[test]
    fn unread_object_across_two_pages_is_not_replaced() {
        let mut doc = Document::new(200.0, 100.0);
        let id = doc.alloc_id();
        doc.insert(
            Some(doc.layers[0].id),
            0,
            Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(90.0, 10.0, 110.0, 20.0)), Default::default()),
        )
        .unwrap();
        let mut page = Document::new(200.0, 100.0);
        for bounds in [Rect::new(91.0, 11.0, 99.0, 19.0), Rect::new(101.0, 11.0, 109.0, 19.0)] {
            let art = Node::path(
                page.alloc_id(),
                vectorcraft_geom::shapes::rectangle(bounds),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            page.insert(Some(page.layers[0].id), page.layers[0].children().unwrap().len(), art).unwrap();
        }
        let (first, second) = (Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.0, 0.0, 200.0, 100.0));
        let frame =
            Frame { page: &page, page_rect: first, rect: first, whole: false, pages: vec![(first, Affine::IDENTITY), (second, Affine::IDENTITY)] };
        let result = replace_unread(&mut doc, &frame, &[(id, vec!["placed files".into()])], &[], &BTreeSet::new(), &mut BTreeSet::new());
        assert_eq!(result.unwrap_err(), "an unread object lies on more than one artboard");
    }

    /// An unread object reaching an artboard that has no page isn't replaced by the page's part of it.
    #[test]
    fn unread_object_on_an_artboard_without_a_page_is_not_replaced() {
        let mut doc = Document::new(100.0, 100.0);
        doc.artboards.push(vectorcraft_doc::Artboard { id: 2, rect: Rect::new(100.0, 0.0, 200.0, 100.0), ..doc.artboards[0].clone() });
        let id = doc.alloc_id();
        doc.insert(
            Some(doc.layers[0].id),
            0,
            Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(90.0, 10.0, 110.0, 20.0)), Default::default()),
        )
        .unwrap();
        let mut page = Document::new(100.0, 100.0);
        let art = Node::path(
            page.alloc_id(),
            vectorcraft_geom::shapes::rectangle(Rect::new(91.0, 11.0, 99.0, 19.0)),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0),
        );
        page.insert(Some(page.layers[0].id), 0, art).unwrap();
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let result = replace_unread(&mut doc, &frame, &[(id, vec!["placed files".into()])], &[], &BTreeSet::new(), &mut BTreeSet::new());
        assert_eq!(result.unwrap_err(), "an unread object lies on an artboard that has no page");

        // Half a point onto it is still on it.
        let narrow = Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(90.0, 10.0, 100.5, 20.0)), Default::default());
        *doc.node_mut(id).unwrap() = narrow;
        let result = replace_unread(&mut doc, &frame, &[(id, vec!["placed files".into()])], &[], &BTreeSet::new(), &mut BTreeSet::new());
        assert_eq!(result.unwrap_err(), "an unread object lies on an artboard that has no page");
    }

    /// A nested placeholder's corner cannot replace its readable sibling.
    #[test]
    fn nested_placeholder_corner_cannot_replace_readable_sibling() {
        let mut doc = Document::new(100.0, 100.0);
        let group_id = doc.alloc_id();
        let placeholder = Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 10.0, 20.0, 20.0)), Default::default());
        let placeholder_id = placeholder.id;
        let sibling = Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(70.0, 70.0, 74.0, 74.0)), Default::default());
        let sibling_id = sibling.id;
        let group = Node::group(group_id, vec![Arc::new(placeholder), Arc::new(sibling)]);
        doc.insert(Some(doc.layers[0].id), 0, group).unwrap();

        let mut page = Document::new(100.0, 100.0);
        let art = Node::path(
            page.alloc_id(),
            vectorcraft_geom::shapes::rectangle(Rect::new(81.0, 81.0, 89.0, 89.0)),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), vectorcraft_color::Paint::None, 0.0),
        );
        page.insert(Some(page.layers[0].id), 0, art).unwrap();
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let result = replace_unread(
            &mut doc,
            &frame,
            &[(group_id, vec!["placed files".into()])],
            &[(placeholder_id, Rect::new(80.0, 80.0, 90.0, 90.0))],
            &BTreeSet::new(),
            &mut BTreeSet::new(),
        );
        assert_eq!(result.unwrap_err(), "an unread object's page art wasn't found in its bounds");
        let group = doc.node(group_id).unwrap();
        assert!(group.children().unwrap().iter().any(|child| child.id == sibling_id));
    }

    /// An alternative box may carry opacity missing from the first box.
    #[test]
    fn alternative_box_can_carry_opacity_when_first_box_cannot() {
        let mut doc = Document::new(100.0, 100.0);
        let id = doc.alloc_id();
        let mut editing = Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 10.0, 20.0, 20.0)), Default::default());
        editing.opacity = 0.5;
        doc.insert(Some(doc.layers[0].id), 0, editing).unwrap();

        let mut page = Document::new(100.0, 100.0);
        for (bounds, opacity) in [(Rect::new(11.0, 11.0, 19.0, 19.0), 1.0), (Rect::new(51.0, 51.0, 59.0, 59.0), 0.5)] {
            let mut art = Node::path(
                page.alloc_id(),
                vectorcraft_geom::shapes::rectangle(bounds),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            art.opacity = opacity;
            page.insert(Some(page.layers[0].id), page.layers[0].children().unwrap().len(), art).unwrap();
        }
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let kinds = replace_unread(
            &mut doc,
            &frame,
            &[(id, vec!["placed files".into()])],
            &[(id, Rect::new(50.0, 50.0, 60.0, 60.0))],
            &BTreeSet::new(),
            &mut BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(kinds, ["placed files (1)"]);
        let replacement = doc.node(id).unwrap();
        let [art] = replacement.children().unwrap().as_slice() else { panic!("{replacement:?}") };
        assert_eq!(art.opacity, 0.5);
        assert_eq!(art.geometric_bounds(), Some(Rect::new(51.0, 51.0, 59.0, 59.0)));
    }

    /// Opacity on one of two page objects in the box is not the unread object's opacity.
    #[test]
    fn opacity_on_one_of_two_page_objects_is_not_identifiable() {
        let mut doc = Document::new(100.0, 100.0);
        let id = doc.alloc_id();
        let mut editing = Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 10.0, 40.0, 40.0)), Default::default());
        editing.opacity = 0.5;
        doc.insert(Some(doc.layers[0].id), 0, editing).unwrap();

        let mut page = Document::new(100.0, 100.0);
        for (bounds, opacity) in [(Rect::new(11.0, 11.0, 19.0, 19.0), 0.5), (Rect::new(21.0, 21.0, 39.0, 39.0), 1.0)] {
            let mut art = Node::path(
                page.alloc_id(),
                vectorcraft_geom::shapes::rectangle(bounds),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            art.opacity = opacity;
            page.insert(Some(page.layers[0].id), page.layers[0].children().unwrap().len(), art).unwrap();
        }
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let result = replace_unread(&mut doc, &frame, &[(id, vec!["placed files".into()])], &[], &BTreeSet::new(), &mut BTreeSet::new());
        assert_eq!(result.unwrap_err(), "an unread object's appearance is not identifiable in the page art");
    }

    /// A claimed first box lets a second unread object use its own corner box.
    #[test]
    fn second_unread_object_tries_corner_after_first_box_is_claimed() {
        let mut doc = Document::new(100.0, 100.0);
        let first = doc.alloc_id();
        let second = doc.alloc_id();
        for (id, bounds) in [(first, Rect::new(10.0, 10.0, 20.0, 20.0)), (second, Rect::new(12.0, 12.0, 22.0, 22.0))] {
            let node = Node::path(id, vectorcraft_geom::shapes::rectangle(bounds), Default::default());
            doc.insert(Some(doc.layers[0].id), doc.layers[0].children().unwrap().len(), node).unwrap();
        }

        let mut page = Document::new(100.0, 100.0);
        for bounds in [Rect::new(14.0, 14.0, 16.0, 16.0), Rect::new(51.0, 51.0, 59.0, 59.0)] {
            let art = Node::path(
                page.alloc_id(),
                vectorcraft_geom::shapes::rectangle(bounds),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            page.insert(Some(page.layers[0].id), page.layers[0].children().unwrap().len(), art).unwrap();
        }
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let unread = [(first, vec!["placed files".into()]), (second, vec!["placed files".into()])];
        let mut one_box_doc = doc.clone();
        let kinds = replace_unread(&mut doc, &frame, &unread, &[(second, Rect::new(50.0, 50.0, 60.0, 60.0))], &BTreeSet::new(), &mut BTreeSet::new())
            .unwrap();
        assert_eq!(kinds, ["placed files (2)"]);
        let first_art = doc.node(first).unwrap().children().unwrap()[0].geometric_bounds();
        let second_art = doc.node(second).unwrap().children().unwrap()[0].geometric_bounds();
        assert_eq!(first_art, Some(Rect::new(14.0, 14.0, 16.0, 16.0)));
        assert_eq!(second_art, Some(Rect::new(51.0, 51.0, 59.0, 59.0)));
        assert_eq!(
            replace_unread(&mut one_box_doc, &frame, &unread, &[], &BTreeSet::new(), &mut BTreeSet::new()).unwrap_err(),
            "one page object matches more than one unread object"
        );
    }

    /// Only the page's type in the page art that replaces an unread object is taken out of the
    /// page's type for the text objects: type in the object's other box, which isn't used, stays.
    #[test]
    fn only_page_type_in_the_chosen_box_is_taken_for_an_unread_object() {
        let mut doc = Document::new(100.0, 100.0);
        let id = doc.alloc_id();
        doc.insert(
            Some(doc.layers[0].id),
            0,
            Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 10.0, 20.0, 20.0)), Default::default()),
        )
        .unwrap();
        let mut page = Document::new(100.0, 100.0);
        let mut nodes = vec![];
        for bounds in [Rect::new(12.0, 12.0, 18.0, 18.0), Rect::new(52.0, 52.0, 58.0, 58.0)] {
            let art = Node::path(
                page.alloc_id(),
                vectorcraft_geom::shapes::rectangle(bounds),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            nodes.push(Arc::new(art.clone()));
            page.insert(Some(page.layers[0].id), page.layers[0].children().unwrap().len(), art).unwrap();
        }
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)] };
        let unread = [(id, vec!["placed files".into()])];
        let mut shown: Vec<Vec<Arc<Node>>> = nodes.iter().map(|n| vec![Arc::clone(n)]).collect();
        let taken = take_unread_type(&doc, Some(&frame), &unread, &[(id, Rect::new(50.0, 50.0, 60.0, 60.0))], &mut shown);
        assert_eq!(taken.iter().flatten().map(|n| n.id).collect::<Vec<_>>(), [nodes[0].id]);
        assert_eq!(shown.iter().flatten().map(|n| n.id).collect::<Vec<_>>(), [nodes[1].id]);
        let mut copied = BTreeSet::new();
        replace_unread(&mut doc, &frame, &unread, &[], &BTreeSet::new(), &mut copied).unwrap();
        assert!(copied.contains(&nodes[0].id) && !copied.contains(&nodes[1].id));
    }
}
