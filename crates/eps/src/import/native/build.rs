use super::compare::{
    MAX_DIFFERENCE, MAX_PAGES_COMPARED, MAX_PIXELS_COMPARED, MAX_SOLID_DIFFERENCE, PageView, compare, comparison_samples, difference, pixels_compared,
};
use super::plan::{made_only, make_slots, plan, readable_all};
use super::slots::{
    ART_LEFT_OUT, Content, DREW_DIFFERENTLY, Filled, MAX_PAIRS, MAX_SLOTS, MAX_TEXT, OUTLINE_REACH, SlotInfo, TEXT_LEFT_OUT, TYPE_LEFT_OUT,
    collect_slots, extent, few, fill_slots, letters, text_objects, words,
};
use super::{
    Affine, Arc, BTreeSet, Document, Node, NodeId, NodeKind, PathData, Rect, Scaling, Story, Structure, Texts, Unit, VariableKind, Vec2, ai,
};
use vectorcraft_doc::PlacedDocument;

/// Which box of the editing copy is the page.
#[derive(Clone, Copy)]
pub(super) enum Page {
    /// An EPS's page is its art's bounding box.
    Art,
    /// A `.ai` is read on its (first and only) artboard.
    Artboard,
}

/// How art moved from a page to its place in the layers' document transforms: its pattern tiles
/// move with it.
pub(super) const PAGE_MOVE: Scaling =
    Scaling { strokes: false, effects: None, keep_type_strokes: false, keep_corners: false, patterns: true, keep_fit_when_flat: false };

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
    /// The pages after the first that show artboards of the layers' document, each with that
    /// artboard (as many as [`MAX_PIXELS_COMPARED`] allows): the layers are compared with them as
    /// with the first.
    pub(super) more: Vec<(Rect, Rect)>,
}

/// The document of the editing copy `data`, with the text (and strokes round it) of `visible`, the
/// file read as its page, put in its text objects → the document and its notes. Without a page
/// (a `.ai` saved without its PDF part), its type is made from the file's text document alone and
/// nothing is compared. `outlined`: the page's type was read as outlines, so it has none to put in
/// the text objects. `unit`: the size in points of a unit of the page's user space (see
/// [`layered_ai`]). `names`: the encoding of names that aren't UTF-8.
pub(super) fn build(
    data: &[u8],
    visible: Option<&Document>,
    page: Page,
    outlined: bool,
    trust: bool,
    unit: f64,
    names: ai::NameEncoding,
) -> Result<(Document, Vec<String>), String> {
    // An EPS (its page is its art) draws a Large Canvas page at full size; a `.ai` keeps its art
    // space, as its PDF part may (#2031).
    let large_canvas = matches!(page, Page::Art);
    let structure = if visible.is_some() { ai::read_over_page(data, large_canvas, names)? } else { ai::read_in(data, large_canvas, names)? };
    build_from(structure, data, visible, page, outlined, trust, unit)
}

/// [`build`] without a page, for a `.ai` whose PDF part can't be read, its first page `page`
/// (width and height, either way round) points large in units of `unit` points: the document is
/// drawn `unit` times as large when its first artboard, or its art's box, has the page's size in
/// those units and not in points, as [`build_from`] decides with a page; else as many times as
/// large as its declared large canvas scale when it has the page's size at that scale. `names`: the
/// encoding of names that aren't UTF-8.
pub(super) fn build_on_page(data: &[u8], page: (f64, f64), unit: f64, names: ai::NameEncoding) -> Result<(Document, Vec<String>), String> {
    let structure = ai::read_in(data, false, names)?;
    let board = structure.artboard.map(|a| structure.to_doc.transform_rect_bbox(a));
    let art = structure.bbox.map(|[a, b, c, d]| structure.to_doc.transform_rect_bbox(Rect::new(a, b, c, d)));
    let (w, h) = page;
    let fits = |r: Option<Rect>, k: f64| {
        r.is_some_and(|r| [(w, h), (h, w)].iter().any(|(a, b)| (r.width() - a / k).abs() <= 0.5 && (r.height() - b / k).abs() <= 0.5))
    };
    let in_units = |r: Option<Rect>, k: f64| k.is_finite() && k > 1.0 && fits(r, k) && !fits(r, 1.0);
    let scale = [unit, structure.canvas_scale].into_iter().find(|&k| in_units(board, k) || (!fits(board, 1.0) && in_units(art, k)));
    let styles = structure.styles_read;
    let (mut doc, notes) = build_from(structure, data, None, Page::Artboard, false, false, 1.0)?;
    if let Some(k) = scale {
        // The graphic styles read from the file are drawn in its units too.
        scale_document(&mut doc, k, styles);
        relayout_wrapped(&mut doc);
    }
    Ok((doc, notes))
}

/// A document read from the editing copy and its notes, or why it can't be read.
pub(super) type Built = Result<(Document, Vec<String>), String>;

/// [`build`] of a `.ai` without importing its PDF part, whose pages (each its trim box in points,
/// width and height) are `pages` and are in units of `unit` points: the document read from
/// `structure` (the editing copy `data` read) with nothing compared with the pages, when the file's
/// text document makes the type of every text object (at least one shows, and none is type that a
/// look draws), no object is unread or has another placement or drawn art that the page would
/// choose, no linked image shows a placeholder, and its artboards are the pages, one for one and of their size, in points or in the pages'
/// units ([`page_scale_alone`]). The document is drawn at that scale. Else `Err(structure)`, for
/// [`build_from`] with the page.
pub(super) fn build_without_page(structure: Structure, data: &[u8], pages: &[(f64, f64)], unit: f64) -> Result<Built, Box<Structure>> {
    let Some(scale) = alone_scale(&structure, data, pages, unit) else {
        return Err(Box::new(structure));
    };
    let styles = structure.styles_read;
    let built = build_from(structure, data, None, Page::Artboard, false, false, 1.0).map(|(mut doc, notes)| {
        if scale != 1.0 {
            // The graphic styles read from the file are drawn in its units too.
            scale_document(&mut doc, scale, styles);
            relayout_wrapped(&mut doc);
        }
        (doc, notes)
    });
    Ok(built)
}

/// The scale [`build_without_page`] draws `structure` (the editing copy `data` read) at on `pages`
/// in units of `unit` points, when it builds it without the page.
pub(super) fn alone_scale(structure: &Structure, data: &[u8], pages: &[(f64, f64)], unit: f64) -> Option<f64> {
    page_scale_alone(structure, pages, unit).filter(|_| nothing_waits_on_page(structure) && type_all_made(structure, data))
}

/// Is nothing in `structure` waiting on the page: no object unread (the page's art takes its
/// place), no placed file with another placement, no live envelope with the drawn art it replaced
/// (the comparison with the page chooses between them), no linked image showing a placeholder
/// where the page has its pixels?
fn nothing_waits_on_page(structure: &Structure) -> bool {
    structure.placeholders.is_empty()
        && structure.unread.is_empty()
        && structure.unread_elsewhere.is_empty()
        && structure.placed_elsewhere.is_empty()
        && structure.envelopes_drawn.is_empty()
}

/// Does the text document of `data` (read as `structure`) make the type of every text object, those
/// that don't show included, with at least one that shows, and none of them type that a look draws?
fn type_all_made(structure: &Structure, data: &[u8]) -> bool {
    let mut infos = vec![];
    collect_slots(&structure.doc.layers, true, &mut infos);
    if !infos.iter().any(|i| i.shown) || infos.iter().any(|i| i.look.is_some()) {
        return false;
    }
    let frames = ai::text_frames(data);
    for info in &mut infos {
        info.art_path = info.story.and_then(|s| frames.get(&(s, info.frame))).map(|p| PathData::from_bezpath(&(structure.to_doc * p.clone())));
    }
    let Some(texts) = Texts::read(data) else { return false };
    let mut read: std::collections::BTreeMap<u32, Option<Arc<Story>>> = Default::default();
    let stories: Vec<Option<Arc<Story>>> = infos
        .iter()
        .map(|i| {
            let n = i.story?;
            read.entry(n).or_insert_with(|| texts.story(n as usize).map(Arc::new)).clone()
        })
        .collect();
    // A slot without its story is made from nothing; past the text `build_from` reads, nothing is
    // made either: it refuses the file.
    if stories.iter().any(Option::is_none) || stories.iter().flatten().map(|s| s.len()).sum::<usize>() > MAX_TEXT {
        return false;
    }
    let template = structure.template.map(|t| ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0));
    make_slots(&infos, &stories, template, structure.to_doc, &mut vec![]).iter().all(Option::is_some)
}

/// The scale at which the artboards of `structure` are the pages `pages` (each its width and height
/// in points), one for one and in order, each within half a point: 1 when they are in points, else
/// `unit` (the pages' `UserUnit`) when they are in those units, as [`page_scale`] finds it with the
/// page. `None` when they aren't the pages, and for a file that declares a large canvas scale (a
/// `.ai` saved without its PDF part reads one at that scale, [`super::ai_alone`]).
fn page_scale_alone(structure: &Structure, pages: &[(f64, f64)], unit: f64) -> Option<f64> {
    let boards = &structure.doc.artboards;
    if boards.is_empty() || boards.len() != pages.len() || structure.canvas_scale != 1.0 {
        return None;
    }
    let fits = |k: f64| boards.iter().zip(pages).all(|(b, (w, h))| (b.rect.width() - w / k).abs() <= 0.5 && (b.rect.height() - h / k).abs() <= 0.5);
    [1.0, unit].into_iter().find(|&k| k.is_finite() && k >= 1.0 && fits(k))
}

/// The areas left out of the comparisons with `page`: the `restyled` looks', and on a PostScript
/// page, which draws nothing for them, the linked images' `placeholders`. A PDF part draws those
/// images' pixels, which the placeholders don't match.
fn left_out_of_comparisons(page: Page, mut restyled: Vec<Rect>, placeholders: Vec<Rect>) -> Vec<Rect> {
    if matches!(page, Page::Art) {
        restyled.extend(placeholders);
    }
    restyled
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
        placeholders,
        placed_elsewhere,
        placed_pages,
        envelopes_drawn,
        styles_read,
        canvas_scale,
        xml_ids,
        bindings,
        bindings_skipped,
        warped_type,
        warnings,
    } = structure;
    let mut restyled = left_out_of_comparisons(page, restyled, placeholders);
    // Editing data in the units of the page's user space ([`in_units`]), or at its declared large
    // canvas scale ([`page_scale`]), is compared with the page scaled down to those units, and the
    // document read from it is scaled up by `unit`.
    let board = artboard.map(|a| to_doc.transform_rect_bbox(a));
    // A page cropped to the art (see below) is in those units as well, when the art's box is and
    // the artboard isn't the page in points.
    let art = bbox.map(|[a, b, c, d]| to_doc.transform_rect_bbox(Rect::new(a, b, c, d)));
    let scaled = page_scale(board, art, visible.filter(|_| matches!(page, Page::Artboard)), unit, canvas_scale);
    let unit = scaled.unwrap_or(unit);
    let small = visible.filter(|_| scaled.is_some()).map(|v| {
        let mut v = v.clone();
        scale_document(&mut v, 1.0 / unit, false);
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
        // A `.ai` read as its PDF part has a page for each artboard it was saved with, in a row:
        // the type on each goes where the layers have that artboard.
        let pages: Vec<Rect> = visible.artboards.iter().map(|a| a.rect).collect();
        let boards = page_boards(&doc, visible, first, &pages);
        // The first page's type goes to the first artboard only when that artboard is the page's
        // (every artboard has a page); otherwise it goes to `rect`, the art's box or the editing
        // data's artboard.
        let first_board = first > 0 || pages.len() == doc.artboards.len();
        let board_of = |i: usize| boards.as_ref()?.get(i).filter(|_| i > 0 || first_board);
        let page_moves: Vec<(Rect, Affine)> = pages
            .iter()
            .enumerate()
            .map(|(i, page)| {
                let move_to_doc = board_of(i).map(|board| Affine::translate(board.rect.origin() - page.origin())).unwrap_or(shift);
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
            let shift = match on.and_then(|(i, page)| Some((board_of(i)?, page))) {
                Some((board, page)) => Affine::translate(board.rect.origin() - page.origin()),
                None => shift,
            };
            for n in object.iter_mut() {
                Arc::make_mut(n).transform(shift, PAGE_MOVE);
            }
        }
        // Pages cropped to the art are copies of the first, and a PDF saved with some of the
        // artboards can hold any of them.
        let more = more_pages(boards.as_deref().filter(|b| !cropped && b.len() == doc.artboards.len()), &pages, comparison_samples(visible));
        // An EPS saved as its artboard has the art outside it left out of its page.
        let artboard_page =
            artboard.is_some_and(|a| [a.x0 - art_box.x0, a.y0 - art_box.y0, a.x1 - art_box.x1, a.y1 - art_box.y1].iter().all(|d| d.abs() <= 1.0));
        let whole = matches!(page, Page::Art) && !artboard_page;
        if whole {
            // Its page is the file's own (its bounding box), as when it comes in as its page: art
            // reaching past the editing data's artboard stays on it (#1102).
            doc.artboards.truncate(1);
            doc.opened_artboard = None;
            if let Some(a) = doc.artboards.first_mut() {
                a.rect = rect;
            }
            drop_guides_off_boards(&mut doc);
        }
        frame = Some(Frame { page: visible, page_rect, rect, whole, pages: page_moves, more });
    }
    let mut infos = vec![];
    collect_slots(&doc.layers, true, &mut infos);
    super::slots::limit_look_slots(&mut doc.layers, &mut infos, shown.len(), outlined);
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
    let mut filled = std::collections::HashMap::new();
    fill_slots(&mut doc, &mut layers, &mut content.into_iter(), &slot_names, &mut empty, &mut made, &mut vec![], &mut filled);
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
    // Area type flows around the wrap objects above it before it is compared with the page.
    resolve_text_wrap(&mut doc)?;
    let bound_text = text_bound_slots(&doc, &bindings, &xml_ids);
    // Without a page to compare the layers with, envelopes of type keep their drawn art: like the
    // other live envelopes, one opens live only where the layers with it draw as the page.
    let warped_type = warped_type.compared(frame.is_some(), &mut notes);
    let typed = settle_warped_type(warped_type, &mut doc, &filled, &threaded_frames, &bound_text, &mut restyled, &mut notes)?;
    // The live envelopes and what takes the place of each again, when the layers with them don't
    // draw as the page, with the note that says so.
    let envelopes_drawn: Vec<(NodeId, Vec<Node>, &str)> = envelopes_drawn
        .into_iter()
        .map(|(id, art)| (id, vec![art], ai::ENVELOPE_DRAWN))
        .chain(typed.into_iter().map(|(id, nodes)| (id, nodes, ai::ENVELOPE_TYPE_DRAWN)))
        .collect();
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
                        let closer = view.difference_on_every_page(&other, frame.rect, reflow).any
                            < view.difference_on_every_page(&doc, frame.rect, reflow).any;
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
                    other => match drawn_or_trusted(&frame, &mut view, &mut doc, &envelopes_drawn, &notes, unseen, trust, why, reflow) {
                        Ok(more) => notes.extend(more),
                        // The objects that draw differently from the page come in as its art, from
                        // the placement that draws closer to it.
                        Err(why) => {
                            let other = other.map(|(_, other, unseen)| (other, unseen));
                            let placements = closer_first(&frame, &mut view, (doc, unseen), other, reflow);
                            let Some((kept, unseen, kept_notes)) = placements.into_iter().find_map(|(base, unseen)| {
                                keep_differing(&base, &frame, &mut view, &threaded_frames, &shown, reflow).map(|(kept, n)| (kept, unseen, n))
                            }) else {
                                return Err(why);
                            };
                            doc = kept;
                            notes.extend(unseen_note(unseen).into_iter().chain(kept_notes));
                        }
                    },
                },
            }
        }
        None => unmark(&mut doc.layers, true, true, &mut vec![]),
    }
    doc.limit_depth_noting(&mut notes);
    bind_variables(&mut doc, &bindings, bindings_skipped, &xml_ids, &filled, &mut notes);
    placed_documents(&mut doc, &placed_pages);
    if small.is_some() {
        scale_document(&mut doc, unit, styles_read);
        relayout_wrapped(&mut doc);
    }
    Ok((doc, notes))
}

/// The linked images of `pages` that are still in `doc`, opacity-mask art included, as placed
/// documents of their page and box, over the same place, each showing the same preview until its
/// file is read. An image bound to an image variable stays a linked image, the object an image
/// variable binds.
fn placed_documents(doc: &mut Document, pages: &[ai::PlacedPage]) {
    if pages.is_empty() {
        return;
    }
    let vars = &doc.variables;
    let image_bound = |id: &NodeId| {
        vars.bindings.get(id).is_some_and(|names| names.iter().any(|v| vars.variable(v).is_some_and(|v| v.kind == VariableKind::Image)))
    };
    let pages: std::collections::HashMap<NodeId, ai::PlacedPage> = pages.iter().filter(|p| !image_bound(&p.id)).map(|p| (p.id, *p)).collect();
    for layer in &mut doc.layers {
        to_placed_documents(Arc::make_mut(layer), &pages);
    }
}

/// [`placed_documents`] in `n` and everything in it.
fn to_placed_documents(n: &mut Node, pages: &std::collections::HashMap<NodeId, ai::PlacedPage>) {
    // A box too small to take the image's pixels to (its transform not finite) keeps the linked image.
    if let Some(page) = pages.get(&n.id)
        && let NodeKind::Image(im) = &n.kind
        && let Some(link) = &im.link
        && let (width, height) = page.size
        && let xf = im.xf * Affine::scale_non_uniform(f64::from(im.width) / width, f64::from(im.height) / height)
        && xf.as_coeffs().iter().all(|v| v.is_finite())
        && xf.determinant().abs() > 1e-12
    {
        let link = vectorcraft_doc::LinkInfo { page: Some(page.page), ..link.clone() };
        let placed = PlacedDocument {
            link,
            key: im.key.clone(),
            bounding: page.bounding,
            page_box: page.page_box,
            width,
            height,
            xf,
            placement: im.placement,
        };
        n.kind = NodeKind::PlacedDocument(Box::new(placed));
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        to_placed_documents(Arc::make_mut(&mut m.art), pages);
    }
    for c in n.children_mut().into_iter().flatten() {
        to_placed_documents(Arc::make_mut(c), pages);
    }
}

/// Bind the objects of `doc` to the variables of its library that `bindings` binds them to, each
/// object named by its XML id (`ids`: the object read with that id, or what `filled` made of a text
/// object's slot: for a visibility variable every object that took the slot's place, for a text
/// variable the one text object among them). An object is bound when it is of the variable's kind:
/// type for a text variable, a linked image for an image variable, any object for a visibility
/// variable. Locked and hidden objects, objects in locked or hidden layers and groups, and objects
/// on locked artboards keep their bindings. Graph variables and
/// variables of unknown kind bind nothing. The note counts the bindings that weren't made, with
/// `skipped` more that the reader left out.
fn bind_variables(
    doc: &mut Document,
    bindings: &[ai::BindingRef],
    skipped: usize,
    ids: &std::collections::HashMap<String, Option<NodeId>>,
    filled: &std::collections::HashMap<NodeId, Filled>,
    notes: &mut Vec<String>,
) {
    let mut skipped = skipped;
    if bindings.is_empty() {
        notes.extend(ai::bindings_note(skipped));
        return;
    }
    let mut bound = vec![];
    for b in bindings {
        let kind = doc.variables.variable(&b.variable).map(|v| v.kind);
        let targets = match (b.target.as_ref().and_then(|t| ids.get(t).copied().flatten()), kind) {
            (Some(id), Some(kind)) => match filled.get(&id) {
                Some(f) if kind == VariableKind::Visibility => f.objects.clone(),
                Some(f) => match f.text.as_slice() {
                    [one] => vec![*one],
                    _ => vec![],
                },
                None => vec![id],
            },
            _ => vec![],
        };
        // Those still in the document: a part of a slot can have been taken out since.
        let targets: Vec<NodeId> = targets.into_iter().filter(|id| doc.node(*id).is_some()).collect();
        let fits = |id: &NodeId| {
            doc.node(*id).is_some_and(|n| match (kind, &n.kind) {
                (Some(VariableKind::Visibility), _) => true,
                (Some(VariableKind::Text), NodeKind::Text(_)) => true,
                (Some(VariableKind::Image), NodeKind::Image(im)) => im.link.is_some(),
                _ => false,
            })
        };
        if !targets.is_empty() && targets.iter().all(fits) {
            bound.push((targets, b.variable.as_str()));
        } else {
            skipped += 1;
        }
    }
    for (targets, variable) in bound {
        for id in targets {
            doc.variables.bind(id, variable);
        }
    }
    notes.extend(ai::bindings_note(skipped));
}

/// The text object slots of `doc` that `bindings` binds to a text variable, each named by its XML id
/// (`ids`).
fn text_bound_slots(doc: &Document, bindings: &[ai::BindingRef], ids: &std::collections::HashMap<String, Option<NodeId>>) -> BTreeSet<NodeId> {
    bindings
        .iter()
        .filter(|b| doc.variables.variable(&b.variable).is_some_and(|v| v.kind == VariableKind::Text))
        .filter_map(|b| b.target.as_ref().and_then(|t| ids.get(t).copied().flatten()))
        .collect()
}

/// Warped type stays type where its live warp draws as its look, and where a text variable binds it
/// (`bound_text`, by slot). Elsewhere the look takes its place, and one note says how many did.
/// `filled` says what each slot became, and the comparisons with the page leave out the boxes in
/// `restyled`. Fails as [`resolve_text_wrap`] does when the looks give the wrap shapes too many
/// anchors. Before that, the envelopes of type open live where they draw as their drawn art, with
/// the type of their text slots (made type, one text object for the slot, not threaded, bound to
/// no text variable and no wrap object); the others keep their drawn art, with a note. Returns
/// each live envelope of type's id with what takes its place again (its hidden type, then its
/// drawn art).
fn settle_warped_type(
    mut warped: ai::WarpedType,
    doc: &mut Document,
    filled: &std::collections::HashMap<NodeId, Filled>,
    threaded: &BTreeSet<NodeId>,
    bound_text: &BTreeSet<NodeId>,
    restyled: &mut Vec<Rect>,
    notes: &mut Vec<String>,
) -> Result<Vec<(NodeId, Vec<Node>)>, String> {
    let made = |slot| match filled.get(&slot).map(|f| f.text.as_slice()).filter(|_| !bound_text.contains(&slot)) {
        Some(&[one]) => Some(one),
        _ => None,
    };
    // An envelope's type: the one text object a slot became (hidden, with nothing else), named as
    // the slot (made type is no longer weighed against the page: the envelope draws it).
    let type_of = |doc: &Document, slot: NodeId| {
        let id = made(slot).filter(|id| !threaded.contains(id) && filled.get(&slot).is_some_and(|f| f.objects == [*id]))?;
        let mut text = doc.node(id).filter(|n| matches!(n.kind, NodeKind::Text(_)) && n.extras().wrap.is_none())?.clone();
        if let Some(name) = text.name.as_deref().and_then(|n| n.strip_prefix(MADE)) {
            text.name = (!name.is_empty()).then(|| name.to_string());
        }
        Some((id, text))
    };
    let typed = warped.settle_envelopes(doc, type_of);
    if !typed.kept.is_empty() && !notes.iter().any(|n| n == ai::ENVELOPE_TYPE_DRAWN) {
        notes.push(ai::ENVELOPE_TYPE_DRAWN.into());
    }
    let replaced = warped.settle(doc, made, threaded, restyled);
    if replaced > 0 {
        notes.push(format!("{}: {replaced}", ai::WARPED_TYPE_DRAWN));
        resolve_text_wrap(doc)?;
    }
    Ok(typed.put_back)
}

/// Most anchors copied into the wrap shapes of a file's area type ([`vectorcraft_doc::text::wrap_work`]).
const MAX_WRAP_WORK: usize = 2_000_000;

/// [`relayout_wrapped`], or an error when the wrap shapes would hold more than [`MAX_WRAP_WORK`]
/// anchors.
fn resolve_text_wrap(doc: &mut Document) -> Result<(), String> {
    if has_wraps(doc) && vectorcraft_doc::text::wrap_work(doc) > MAX_WRAP_WORK {
        return Err("it has too many text objects".into());
    }
    relayout_wrapped(doc);
    Ok(())
}

/// Does an object of `doc` have text wrap options, or a text object wrap shapes?
fn has_wraps(doc: &Document) -> bool {
    let mut wraps = false;
    doc.walk(|n| wraps |= n.extras().wrap.is_some() || matches!(&n.kind, NodeKind::Text(t) if !t.wrap.is_empty()));
    wraps
}

/// Resolve the wrap shapes of the area type in `doc` that has wrap objects above it (see
/// [`vectorcraft_doc::text::resolve_wrap`]); the type whose shapes changed is laid out again.
/// Nothing is resolved when the shapes would hold more than [`MAX_WRAP_WORK`] anchors (such a
/// file isn't read from its layers: see [`resolve_text_wrap`]).
fn relayout_wrapped(doc: &mut Document) {
    if !has_wraps(doc) || vectorcraft_doc::text::wrap_work(doc) > MAX_WRAP_WORK {
        return;
    }
    for id in vectorcraft_doc::text::resolve_wrap(None, doc) {
        if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
            t.cached_bounds = None;
            t.cached_baselines.clear();
        }
    }
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
                copy.transform(*shift, PAGE_MOVE);
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

/// Most objects of the layers that come in as the page's art where they draw differently from it.
const MAX_DIFFERING: usize = 64;

/// Most pairs of a patch where the layers and the page differ and a top-level object that may hold
/// it, weighed to find the objects that come in as the page's art.
const MAX_PATCH_PAIRS: usize = 4_000_000;

/// The layers as read and at the placed files' other placement (each with its count of text
/// objects the page doesn't draw), the one that draws closer to the page first. `reflow` as in
/// [`PageView::difference`].
fn closer_first(
    frame: &Frame<'_>,
    view: &mut PageView,
    doc: (Document, usize),
    other: Option<(Document, usize)>,
    reflow: bool,
) -> Vec<(Document, usize)> {
    match other {
        Some(other)
            if view.difference_on_every_page(&other.0, frame.rect, reflow).any < view.difference_on_every_page(&doc.0, frame.rect, reflow).any =>
        {
            vec![other, doc]
        }
        Some(other) => vec![doc, other],
        None => vec![doc],
    }
}

/// `doc`, whose layers draw differently from the page (an object in another color, or one that only
/// one of them draws), with each of its top-level objects that holds such a difference replaced by the
/// page's art in its bounds, when that makes the two look alike → the document and its notes: the
/// comparison's, and how many objects were replaced on each layer (by name, in the layers' order).
/// `None` when only edges or type differ, a difference lies outside every object, more than half
/// of the objects (or more than [`MAX_DIFFERING`]) hold one, one of them holds type or a wrap
/// object, the page's art can't replace one of them ([`replace_unread`]), the page's type of a text
/// object that stays is copied into a replacement, or the two still differ. `shown` is the page's
/// type of each text object that shows. `reflow` as in [`PageView::difference`].
fn keep_differing(
    doc: &Document,
    frame: &Frame<'_>,
    view: &mut PageView,
    threaded_frames: &BTreeSet<NodeId>,
    shown: &[Vec<Arc<Node>>],
    reflow: bool,
) -> Option<(Document, Vec<String>)> {
    /// The top-level objects that show and print (the outermost objects under a layer or
    /// sublayer), with the name of that layer or sublayer as the Layers panel shows it and their
    /// bounds.
    fn top_level(nodes: &[Arc<Node>], layer: &str, out: &mut Vec<(String, NodeId, Rect)>) {
        for n in nodes.iter().filter(|n| n.visible) {
            match &n.kind {
                NodeKind::Layer { printable: false, .. } => {}
                NodeKind::Layer { .. } => {
                    let name = Some(n.display_name()).filter(|name| !name.is_empty()).unwrap_or_else(|| format!("<{}>", n.kind_label()));
                    top_level(n.children().map_or(&[], Vec::as_slice), &name, out);
                }
                NodeKind::Path { guide: true, .. } => {}
                _ => {
                    if let Some(b) = n.visual_bounds().filter(|b| b.area().is_finite() && b.area() > 0.0) {
                        out.push((layer.to_string(), n.id, b));
                    }
                }
            }
        }
    }

    // Where only spread edges or type differ, or only the art around the page, no object is
    // replaced.
    let (diff, regions) = view.solid_regions_on_every_page(doc, reflow)?;
    if diff.solid <= MAX_SOLID_DIFFERENCE {
        return None;
    }
    let mut objects = vec![];
    top_level(&doc.layers, "", &mut objects);
    let pages = view.regions(frame.rect);
    objects.retain(|(_, _, b)| pages.iter().any(|page| !b.intersect(*page).is_zero_area()));
    if regions.len().saturating_mul(objects.len()) > MAX_PATCH_PAIRS {
        return None;
    }
    // Each patch is the smallest object's that holds it.
    let most = MAX_DIFFERING.min(objects.len() / 2);
    let mut targets = BTreeSet::new();
    for region in regions {
        let (_, id, _) = objects.iter().filter(|(_, _, b)| inside(region, *b)).min_by(|a, b| a.2.area().total_cmp(&b.2.area()))?;
        targets.insert(*id);
        if targets.len() > most {
            return None;
        }
    }
    if targets.is_empty() {
        return None;
    }
    // A replaced wrap object keeps its wrap options, and the area type is wrapped around the page's
    // art that replaces it. An object inside a replaced group can't keep them, so such a group isn't
    // replaced. Neither is an object that holds type: the page's art that replaces it would hold
    // type the page draws for other text objects too.
    let mut wraps = vec![];
    for id in &targets {
        let node = doc.node(*id)?;
        let mut held = matches!(node.kind, NodeKind::Text(_));
        for child in node.children().into_iter().flatten() {
            child.walk(&mut |n| held |= n.extras().wrap.is_some() || matches!(n.kind, NodeKind::Text(_)));
        }
        if held {
            return None;
        }
        if let Some(wrap) = node.extras().wrap {
            wraps.push((*id, wrap));
        }
    }
    let mut kept = doc.clone();
    let replace: Vec<(NodeId, Vec<String>)> = targets.iter().map(|id| (*id, vec![])).collect();
    let mut copied = BTreeSet::new();
    replace_unread(&mut kept, frame, &replace, &[], threaded_frames, &mut copied).ok()?;
    // A text object that stays draws its page type, so a copy of that type would draw it twice.
    if shown.iter().flatten().any(|n| copied.contains(&n.id)) {
        return None;
    }
    for (id, wrap) in wraps {
        kept.node_mut(id)?.extras_mut().wrap = Some(wrap);
    }
    resolve_text_wrap(&mut kept).ok()?;
    let note = compare(frame, view, &kept, reflow).ok()?;
    let mut layers: Vec<(String, usize)> = vec![];
    for (layer, _, _) in objects.iter().filter(|(_, id, _)| targets.contains(id)) {
        match layers.iter_mut().find(|(name, _)| name == layer) {
            Some((_, count)) => *count += 1,
            None => layers.push((layer.clone(), 1)),
        }
    }
    let layers: Vec<String> = layers.into_iter().map(|(layer, count)| format!("{layer} ({count})")).collect();
    Some((kept, note.into_iter().chain([format!("{DREW_DIFFERENTLY}: {}", layers.join(", "))]).collect()))
}

/// Copy page drawing IDs, including mask art outside the ordinary child tree.
pub(super) fn reid_page_art(doc: &mut Document, source: &Node) -> Node {
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
    // where every made one draws: that has its limits (the slots of type that looks draw are within
    // them, see `build_from`).
    let open = infos.iter().zip(&made).filter(|(i, m)| i.look.is_none() && m.is_none()).count();
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
    drawn: &[(NodeId, Vec<Node>, &str)],
    notes: &[String],
    unseen: usize,
    trust: bool,
    why: String,
    reflow: bool,
) -> Result<Vec<String>, String> {
    let unseen_note = |n: usize| (n > 0).then(|| format!("{n} {UNSEEN_TEXT}"));
    let mut put_back = doc.clone();
    // Each envelope still there, in place of the art it took the place of.
    let there: Vec<&(NodeId, Vec<Node>, &str)> = drawn.iter().filter(|(id, ..)| put_back.node(*id).is_some()).collect();
    let any = !there.is_empty() && put_back.insert_beside(there.iter().map(|(id, art, _)| (*id, art.clone())).collect(), true).is_ok();
    if any {
        let more = prune_unseen(frame, view, &mut put_back, reflow);
        if let Ok(note) = compare(frame, view, &put_back, reflow) {
            *doc = put_back;
            let mut envelope: Vec<String> = vec![];
            for (.., say) in there {
                if !notes.iter().chain(&envelope).any(|n| n == say) {
                    envelope.push(say.to_string());
                }
            }
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
    // Area type wrapped around a moved image flows around it where it now is.
    if moved {
        relayout_wrapped(&mut doc);
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

/// Is the editing data in units of `unit` points beside `page` ([`in_units`])? Its first artboard
/// `board` is, or, when that isn't the page in points, its art's box `art` is (a page cropped to the
/// art).
fn in_units_or_art(board: Option<Rect>, art: Option<Rect>, page: &Document, unit: f64) -> bool {
    in_units(board, page, unit) || (!same_size(board, page) && in_units(art, page, unit))
}

/// The size in points of a unit of the editing data whose first artboard is `board` and whose
/// art's box is `art` (the document's coordinates), read beside `page`: `unit` (the page's
/// `UserUnit`) when the data is in those units ([`in_units_or_art`]), else `canvas_scale` (its
/// declared large canvas scale) when the data is at that scale; `None` when it is in points or
/// there is no page.
fn page_scale(board: Option<Rect>, art: Option<Rect>, page: Option<&Document>, unit: f64, canvas_scale: f64) -> Option<f64> {
    let page = page?;
    [unit, canvas_scale].into_iter().find(|&k| in_units_or_art(board, art, page, k))
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

/// The most artboards [`shown_artboard`] weighs for the first page, and the most [`page_boards`]
/// weighs for all the pages after it together, a render each.
const MAX_BOARDS_WEIGHED: usize = 16;

/// Does artboard `a` have the size of `page`?
fn board_fits(a: &vectorcraft_doc::Artboard, page: &Rect) -> bool {
    (a.rect.width() - page.width()).abs() <= 0.5 && (a.rect.height() - page.height()).abs() <= 0.5
}

/// The last of the artboards `boards` that each page of `pages` can show when every page shows
/// one, in the pages' order. A page after the first shows an artboard of its size. Working from
/// the last page back, each page's entry is the last artboard of its size before the next page's
/// entry. The first page's entry is the artboard before the second page's entry, or the last
/// artboard when there is one page. `None` when the pages after the first have no such artboards.
fn last_boards(boards: &[vectorcraft_doc::Artboard], pages: &[Rect]) -> Option<Vec<usize>> {
    let mut last = vec![0; pages.len()];
    let mut end = boards.len();
    for (i, page) in pages.iter().enumerate().skip(1).rev() {
        let j = (1..end).rev().find(|&j| boards.get(j).is_some_and(|a| board_fits(a, page)))?;
        *last.get_mut(i)? = j;
        end = j;
    }
    if let Some(l) = last.first_mut() {
        *l = end.checked_sub(1)?;
    }
    Some(last)
}

/// Which of the artboards of `doc` (the layers' document) the page `page_rect` of `visible` shows,
/// when `visible` has fewer pages than `doc` has artboards (a PDF saved with a range of them): of
/// those of the page's size (`fits`) that leave an artboard for each later page ([`last_boards`]),
/// the one whose art draws most like the page, the first of them on a tie. Else (and for an EPS,
/// whose page is its art) the first artboard (0).
fn shown_artboard(doc: &Document, visible: &Document, page: Page, page_rect: Rect, fits: impl Fn(&Rect) -> bool) -> usize {
    if matches!(page, Page::Art) || visible.artboards.len() >= doc.artboards.len() {
        return 0;
    }
    let pages: Vec<Rect> = visible.artboards.iter().map(|a| a.rect).collect();
    let last = last_boards(&doc.artboards, &pages).and_then(|l| l.first().copied()).unwrap_or(usize::MAX);
    let sized: Vec<usize> =
        doc.artboards.iter().enumerate().filter(|(i, a)| *i <= last && fits(&a.rect)).map(|(i, _)| i).take(MAX_BOARDS_WEIGHED).collect();
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

/// The pages after the first, each with the artboard it shows ([`page_boards`]), as [`Frame::more`]
/// holds them: none when the pages don't show the artboards. They stop before the page that would
/// take the pixels the comparisons draw (`samples`, [`comparison_samples`]), the first page's
/// included, past [`MAX_PIXELS_COMPARED`], and after [`MAX_PAGES_COMPARED`] pages.
fn more_pages(boards: Option<&[vectorcraft_doc::Artboard]>, pages: &[Rect], samples: u32) -> Vec<(Rect, Rect)> {
    let Some(boards) = boards else { return vec![] };
    let mut left = MAX_PIXELS_COMPARED;
    pages
        .iter()
        .zip(boards)
        .map_while(|(page, board)| {
            left = left.checked_sub(pixels_compared(*page, samples))?;
            Some((*page, board.rect))
        })
        .take(MAX_PAGES_COMPARED)
        .skip(1)
        .collect()
}

/// The artboards (or other areas) of `pages` that `bounds` lies on, with `margin` (points) round it:
/// the pages whose drawings taking it out can change.
fn pages_touched(bounds: Rect, pages: &[Rect], margin: f64) -> Vec<usize> {
    let b = bounds.inflate(margin, margin);
    pages.iter().enumerate().filter(|(_, p)| !b.intersect(**p).is_zero_area()).map(|(i, _)| i).collect()
}

/// The artboards of the layers' document (`doc`) that the pages of `visible` (`pages`) show, one a
/// page, the first page's being `first`, each of its page's size. The artboards are in the pages'
/// order. A page after the first shows an artboard after the previous page's, and only one that
/// leaves an artboard for each later page ([`last_boards`]). When several artboards can be the
/// page's (a PDF saved with some of them), the page shows the one whose art draws most like it,
/// the first of them on a tie. At most [`MAX_BOARDS_WEIGHED`] artboards are weighed for all the
/// later pages together, each page's candidates in order: a page takes the best of its candidates
/// that were weighed, or its first candidate when none of them were. `None` when the pages have no
/// such artboards.
fn page_boards(doc: &Document, visible: &Document, first: usize, pages: &[Rect]) -> Option<Vec<vectorcraft_doc::Artboard>> {
    let boards = &doc.artboards;
    let same = |j: usize, p: &Rect| boards.get(j).is_some_and(|a| board_fits(a, p));
    let last = last_boards(boards, pages)?;
    if !pages.first().is_some_and(|p| same(first, p)) || last.first().is_none_or(|&l| first > l) {
        return None;
    }
    let mut shown = vec![boards.get(first)?.clone()];
    let (mut after, mut weighed) = (first + 1, 0);
    for (i, page) in pages.iter().enumerate().skip(1) {
        let sized: Vec<usize> = (after..=*last.get(i)?).filter(|&j| same(j, page)).collect();
        let j = match sized.as_slice() {
            [] => return None,
            [only] => *only,
            many => {
                let weigh: Vec<usize> = many.iter().copied().take(MAX_BOARDS_WEIGHED.saturating_sub(weighed)).collect();
                weighed += weigh.len();
                let mut best: Option<(usize, f64)> = None;
                for (j, a) in weigh.iter().filter_map(|&j| Some((j, boards.get(j)?))) {
                    let d = difference(visible, *page, doc, a.rect).any;
                    if best.is_none_or(|(_, b)| d < b) {
                        best = Some((j, d));
                    }
                }
                best.map_or(many.first().copied(), |(j, _)| Some(j))?
            }
        };
        shown.push(boards.get(j)?.clone());
        after = j + 1;
    }
    Some(shown)
}

/// Drops the guides of artboards `doc` no longer has: an artboard guide goes with its artboard.
fn drop_guides_off_boards(doc: &mut Document) {
    let kept: Vec<u32> = doc.artboards.iter().map(|a| a.id).collect();
    doc.guides.retain(|g| g.artboard.is_none_or(|id| kept.contains(&id)));
}

/// Draw `doc`'s layers, artboards and guides `k` times as large, from the origin, with their
/// strokes, effects and pattern tiles, and with `styles` the strokes and effects of its graphic
/// styles.
pub(crate) fn scale_document(doc: &mut Document, k: f64, styles: bool) {
    let a = Affine::scale(k);
    let scaling = Scaling { strokes: true, effects: Some(vectorcraft_effects::scale_effect), patterns: true, ..Default::default() };
    for layer in &mut doc.layers {
        let layer = Arc::make_mut(layer);
        layer.transform(a, scaling);
        scale_wrap_offsets(layer, k);
    }
    for board in &mut doc.artboards {
        board.rect = a.transform_rect_bbox(board.rect);
    }
    for guide in &mut doc.guides {
        guide.pos *= k;
    }
    if styles {
        for g in doc.graphic_styles.iter_mut() {
            g.appearance.scale_strokes(k);
            g.appearance.scale_effects(k, vectorcraft_effects::scale_effect);
        }
    }
}

/// The text wrap offsets of `n` and what it holds, times `k` (within the -1000 to 1000 points
/// they may be).
fn scale_wrap_offsets(n: &mut Node, k: f64) {
    if let Some(w) = n.extras_if_any().and_then(|e| e.wrap.as_mut()) {
        let v = w.offset * k;
        if !v.is_nan() {
            w.offset = v.clamp(-1000.0, 1000.0);
        }
    }
    if let Some(children) = n.children_mut() {
        for c in children {
            scale_wrap_offsets(Arc::make_mut(c), k);
        }
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

/// The text objects made from the file's text document that show and print on a page's area (the
/// frame's first page or another one, [`Frame::more`]), where the page has no type to match them, and
/// that the layers look no worse without on every page they lie on, are not drawn by the app that
/// wrote the file (which paints every shown text object; it keeps the type of what was turned to
/// outlines): they are taken out. How many. `reflow`: the type is all laid out again from the text
/// document (see [`compare`]).
pub(super) fn prune_unseen(frame: &Frame<'_>, view: &mut PageView, doc: &mut Document, reflow: bool) -> usize {
    let mut found = vec![];
    unmark(&mut doc.layers, true, true, &mut found);
    if found.is_empty() || found.len() > MAX_WEIGHED {
        return 0;
    }
    let pages = view.regions(frame.rect);
    // Taking a text object out changes only what is drawn in its box. When the art on a page would
    // still differ from it by more than `compare` allows with every candidate gone, pruning doesn't
    // make that page pass: the objects on it aren't drawn again one by one (#758).
    let margin = OUTLINE_REACH + 1.0;
    let hopeless: Vec<bool> = view
        .differences(doc, frame.rect, reflow)
        .iter()
        .zip(&pages)
        .map(|(with, rect)| {
            let boxes: f64 = found.iter().filter_map(|(_, b)| *b).map(|b| b.inflate(margin, margin).intersect(*rect).area().max(0.0)).sum();
            with.weighed(reflow) - boxes / rect.area().max(1e-9) > MAX_DIFFERENCE
        })
        .collect();
    // Weighed by colour, not only lightness: a text object's red stroke over a dark red one is no
    // less there for being about as light (#1102). The view keeps each page's drawing and share: a
    // removal has only the pages in `on` drawn again.
    let mut removed = 0;
    for (id, bounds) in found {
        // A removal before it can have let its area type flow differently.
        let bounds = if removed > 0 { doc.node(id).and_then(|n| n.visual_bounds()) } else { bounds };
        let Some(b) = bounds else { continue };
        let mut lies = pages_touched(b, &pages, 0.0);
        if lies.is_empty() {
            continue;
        }
        // The pages its strokes and effects can reach past its box are drawn again too.
        let mut on = pages_touched(b, &pages, margin);
        // Area type that flows around it can reach other pages: they are all weighed.
        if has_wraps(doc) {
            (lies, on) = ((0..pages.len()).collect(), (0..pages.len()).collect());
        }
        if lies.iter().any(|i| hopeless.get(*i).copied().unwrap_or(true)) {
            continue;
        }
        let mut without = doc.clone();
        if !take_out(&mut without.layers, id, true) {
            continue;
        }
        // Area type that wrapped around the text object flows without it.
        relayout_wrapped(&mut without);
        let shares = view.weigh_without(doc, &without, &on);
        if shares.len() == on.len() && shares.iter().all(|(_, before, after)| after - before <= UNSEEN_MARGIN) {
            take_out(&mut doc.layers, id, false);
            relayout_wrapped(doc);
            // Only the pages in `on` can draw differently (every page when type wraps).
            view.adopt(doc);
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod candidate_tests {
    use super::*;

    use vectorcraft_doc::{ImageBlob, ImageObject};

    /// The pages compared stop where their drawings would take more than the pixels allowed.
    #[test]
    fn the_pages_compared_stop_at_the_pixel_bound() {
        let pages: Vec<Rect> = (0..1000).map(|i| Rect::new(f64::from(i) * 700.0, 0.0, f64::from(i) * 700.0 + 612.0, 792.0)).collect();
        let doc = Document::new(612.0, 792.0);
        let boards: Vec<vectorcraft_doc::Artboard> = pages
            .iter()
            .map(|page| {
                let mut board = doc.artboards[0].clone();
                board.rect = *page;
                board
            })
            .collect();
        let more = more_pages(Some(&boards), &pages, 1);
        let each = pixels_compared(pages[0], 1);
        assert_eq!(more.len() + 1, (MAX_PIXELS_COMPARED / each) as usize);
        assert!(more.len() >= 64, "{}", more.len());
        assert_eq!(more_pages(Some(&boards[..3]), &pages[..3], 1).len(), 2);
        // Pages drawn with 4 × 4 samples a pixel take 16 times the pixels.
        assert_eq!(more_pages(Some(&boards), &pages, 4).len() + 1, (MAX_PIXELS_COMPARED / (each * 16)) as usize);
        // Tiny pages stop at the page count.
        let tiny: Vec<Rect> = (0..1000).map(|i| Rect::new(f64::from(i) * 2.0, 0.0, f64::from(i) * 2.0 + 1.0, 1.0)).collect();
        let tiny_boards: Vec<vectorcraft_doc::Artboard> =
            boards.iter().zip(&tiny).map(|(b, r)| vectorcraft_doc::Artboard { rect: *r, ..b.clone() }).collect();
        assert_eq!(more_pages(Some(&tiny_boards), &tiny, 1).len() + 1, MAX_PAGES_COMPARED);
    }

    /// A page after the first doesn't take an artboard that a later page of another size needs,
    /// even one that draws more like it.
    #[test]
    fn a_later_page_leaves_an_artboard_of_each_later_pages_size() {
        let square = |doc: &mut Document, x: f64| {
            let art = Node::path(
                doc.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0)),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            let layer = doc.layers[0].id;
            doc.insert(Some(layer), 0, art).unwrap();
        };
        let board = |doc: &Document, x0: f64, width: f64| vectorcraft_doc::Artboard {
            rect: Rect::new(x0, 0.0, x0 + width, 100.0),
            ..doc.artboards[0].clone()
        };
        // Artboards of 100, 100, 200, 100 and 100 points, a square on the fourth.
        let mut doc = Document::new(100.0, 100.0);
        doc.artboards = [(0.0, 100.0), (150.0, 100.0), (300.0, 200.0), (550.0, 100.0), (700.0, 100.0)].map(|(x, w)| board(&doc, x, w)).to_vec();
        square(&mut doc, 560.0);
        // Pages of 100, 100 and 200 points, the second with the fourth artboard's square.
        let mut page = Document::new(100.0, 100.0);
        let pages = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(120.0, 0.0, 220.0, 100.0), Rect::new(240.0, 0.0, 440.0, 100.0)];
        page.artboards = pages.iter().map(|r| board(&page, r.x0, r.width())).collect();
        square(&mut page, 130.0);
        let shown = page_boards(&doc, &page, 0, &pages).map(|b| b.iter().map(|a| a.rect.x0).collect::<Vec<_>>());
        assert_eq!(shown, Some(vec![0.0, 150.0, 300.0]));
    }

    /// The pages after the first weigh at most [`MAX_BOARDS_WEIGHED`] artboards together: a page
    /// takes the best of its candidates that were weighed, and a page left with none to weigh
    /// takes its first candidate, even when a later candidate draws like it.
    #[test]
    fn the_later_pages_weigh_a_bounded_number_of_artboards() {
        let square = |doc: &mut Document, x: f64| {
            let art = Node::path(
                doc.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0)),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            let layer = doc.layers[0].id;
            doc.insert(Some(layer), 0, art).unwrap();
        };
        let board = |doc: &Document, x0: f64, width: f64| vectorcraft_doc::Artboard {
            rect: Rect::new(x0, 0.0, x0 + width, 100.0),
            ..doc.artboards[0].clone()
        };
        // Artboards 0 to 17 are 100 points wide, 150 apart; artboards 18 and 19 are 200 points
        // wide. The second page's square is on artboard 17 and the third page's on artboard 19.
        let n = MAX_BOARDS_WEIGHED + 2;
        let mut doc = Document::new(100.0, 100.0);
        let mut boards: Vec<vectorcraft_doc::Artboard> = (0..n).map(|i| board(&doc, i as f64 * 150.0, 100.0)).collect();
        let wide = n as f64 * 150.0;
        boards.push(board(&doc, wide, 200.0));
        boards.push(board(&doc, wide + 250.0, 200.0));
        doc.artboards = boards;
        square(&mut doc, (n - 1) as f64 * 150.0 + 10.0);
        square(&mut doc, wide + 260.0);
        let mut page = Document::new(100.0, 100.0);
        let pages = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(120.0, 0.0, 220.0, 100.0), Rect::new(240.0, 0.0, 440.0, 100.0)];
        page.artboards = pages.iter().map(|r| board(&page, r.x0, r.width())).collect();
        square(&mut page, 130.0);
        square(&mut page, 250.0);
        // The second page weighs artboards 1 to 16, which draw alike, and takes artboard 1. The
        // third page has no weighing left and takes artboard 18.
        let shown = page_boards(&doc, &page, 0, &pages).map(|b| b.iter().map(|a| a.rect.x0).collect::<Vec<_>>());
        assert_eq!(shown, Some(vec![0.0, 150.0, wide]));
    }

    /// The first page of a PDF of some artboards shows an artboard that leaves one for each later
    /// page, even when a later artboard draws more like it.
    #[test]
    fn the_first_page_leaves_an_artboard_for_each_later_page() {
        let square = |doc: &mut Document, x: f64| {
            let art = Node::path(
                doc.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(x, 10.0, x + 20.0, 30.0)),
                vectorcraft_doc::Appearance::basic(
                    vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK),
                    vectorcraft_color::Paint::None,
                    0.0,
                ),
            );
            let layer = doc.layers[0].id;
            doc.insert(Some(layer), 0, art).unwrap();
        };
        let board = |doc: &Document, x0: f64| vectorcraft_doc::Artboard { rect: Rect::new(x0, 0.0, x0 + 100.0, 100.0), ..doc.artboards[0].clone() };
        // Three artboards of one size, a square on the third.
        let mut doc = Document::new(100.0, 100.0);
        doc.artboards = [0.0, 150.0, 300.0].map(|x| board(&doc, x)).to_vec();
        square(&mut doc, 310.0);
        // Two pages, the square on the first.
        let mut page = Document::new(100.0, 100.0);
        let pages = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(120.0, 0.0, 220.0, 100.0)];
        page.artboards = pages.iter().map(|r| board(&page, r.x0)).collect();
        square(&mut page, 10.0);
        let fits = |r: &Rect| (r.width() - 100.0).abs() <= 0.5 && (r.height() - 100.0).abs() <= 0.5;
        let first = shown_artboard(&doc, &page, Page::Artboard, pages[0], fits);
        assert_eq!(first, 0);
        let shown = page_boards(&doc, &page, first, &pages).map(|b| b.iter().map(|a| a.rect.x0).collect::<Vec<_>>());
        assert_eq!(shown, Some(vec![0.0, 150.0]));
    }

    /// A linked image showing a placeholder (`XF`) waits on the page, whose PDF part draws its
    /// pixels: the layers aren't used without it.
    #[test]
    fn a_linked_image_placeholder_waits_on_the_page() {
        let raster = "%AI5_File:\n%AI5_BeginRaster\n(photo.tif) 0 XG\n[ 1 0 0 1 0 4 ] 0 0 4 4 4 4 8 3 0 0 0 0 XF\n%AI5_EndRaster\n";
        let data = format!(
            "%!PS-Adobe-3.0 \n%%BoundingBox: 0 0 100 100\n%AI3_Cropmarks: 0 0 100 100\n%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n{raster}LB\n%AI5_EndLayer--\n%%Trailer\n"
        );
        let structure = ai::read(data.as_bytes(), false).unwrap();
        assert_eq!(structure.placeholders.len(), 1);
        assert!(!nothing_waits_on_page(&structure));
    }

    /// The pages a box touches with a margin round it include those it only comes near.
    #[test]
    fn the_pages_within_reach_of_a_box_count_as_touched() {
        let pages = [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(110.0, 0.0, 210.0, 100.0)];
        let near_the_second = Rect::new(60.0, 40.0, 106.0, 60.0);
        assert_eq!(pages_touched(near_the_second, &pages, 0.0), vec![0]);
        assert_eq!(pages_touched(near_the_second, &pages, OUTLINE_REACH + 1.0), vec![0, 1]);
    }

    /// Layers read in the units of a page scaled up to points take their wrap offsets with their art,
    /// within ±1000 points.
    #[test]
    fn scaling_the_document_scales_its_wrap_offsets() {
        let mut doc = Document::new(100.0, 100.0);
        let mut ids = vec![];
        for (x, offset) in [(0.0, 6.0), (20.0, 200.0), (40.0, -150.0)] {
            let id = doc.alloc_id();
            let mut n = Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Default::default());
            n.extras_mut().wrap = Some(vectorcraft_doc::TextWrap { offset, invert: true });
            doc.insert(Some(doc.layers[0].id), 0, n).unwrap();
            ids.push(id);
        }
        scale_document(&mut doc, 10.0, false);
        let offsets: Vec<_> = ids.iter().map(|id| doc.node(*id).and_then(|n| n.extras().wrap)).collect();
        let wrap = |offset| Some(vectorcraft_doc::TextWrap { offset, invert: true });
        assert_eq!(offsets, [wrap(60.0), wrap(1000.0), wrap(-1000.0)]);
    }

    /// Area type in `doc`'s first layer at `index`, in the frame `r`.
    fn insert_area(doc: &mut Document, index: usize, r: Rect) -> NodeId {
        let mut t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::ZERO, "Wrapped", Default::default());
        t.kind = vectorcraft_doc::TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(r) };
        let id = doc.alloc_id();
        doc.insert(Some(doc.layers[0].id), index, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap()
    }

    /// A wrap object on top of `doc`'s first layer: a closed path of `anchors` anchors in `r`.
    fn insert_wrap(doc: &mut Document, r: Rect, anchors: usize) -> NodeId {
        let mut bp = kurbo::BezPath::new();
        bp.move_to((r.x0, r.y0));
        for i in 1..anchors {
            let t = i as f64 / anchors as f64;
            bp.line_to((r.x0 + r.width() * t, if i % 2 == 0 { r.y0 } else { r.y1 }));
        }
        bp.close_path();
        let id = doc.alloc_id();
        let mut n = Node::path(id, PathData::from_bezpath(&bp), Default::default());
        n.extras_mut().wrap = Some(vectorcraft_doc::TextWrap::default());
        let top = doc.layers[0].children().map_or(0, Vec::len);
        doc.insert(Some(doc.layers[0].id), top, n).unwrap()
    }

    fn wrap_shapes(doc: &Document, id: NodeId) -> usize {
        match doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => t.wrap.len(),
            _ => panic!("area type"),
        }
    }

    /// A file whose wrap shapes would copy more than `MAX_WRAP_WORK` anchors isn't read from its
    /// layers; one below that is.
    #[test]
    fn wrap_shapes_of_too_many_text_objects_are_refused() {
        for (texts, ok) in [(150, true), (250, false)] {
            let mut doc = Document::new(100.0, 100.0);
            let ids: Vec<NodeId> = (0..texts).map(|_| insert_area(&mut doc, 0, Rect::new(0.0, 0.0, 100.0, 100.0))).collect();
            insert_wrap(&mut doc, Rect::new(40.0, 0.0, 60.0, 100.0), 10_000);
            assert!(vectorcraft_doc::text::wrap_work(&doc) >= texts * 10_000);
            assert_eq!(resolve_text_wrap(&mut doc).is_ok(), ok, "{texts} text objects");
            // Nor is anything copied on the way, as the placed files' other placement does.
            relayout_wrapped(&mut doc);
            assert!(ids.iter().all(|id| wrap_shapes(&doc, *id) == usize::from(ok)), "{texts} text objects");
        }
    }

    /// Warped type whose look takes its place, with the type's wrap setting, is held to the same
    /// limit on wrap shapes: a file whose looks would copy more than `MAX_WRAP_WORK` anchors isn't
    /// read from its layers.
    #[test]
    fn warped_type_replaced_by_its_look_is_held_to_the_wrap_limit() {
        for (texts, ok) in [(150, true), (250, false)] {
            let mut doc = Document::new(100.0, 100.0);
            (0..texts).for_each(|_| _ = insert_area(&mut doc, 0, Rect::new(0.0, 0.0, 100.0, 100.0)));
            let t = vectorcraft_doc::TextObject::point(vectorcraft_geom::Point::new(50.0, 50.0), "TILT", Default::default());
            let mut text = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
            let mut appearance = vectorcraft_doc::Appearance::default();
            appearance.effects.push(vectorcraft_doc::Effect { id: "warp.arc".into(), params: serde_json::json!({"bend": 40.0}), visible: true });
            text.appearance = appearance.into();
            text.extras_mut().wrap = Some(vectorcraft_doc::TextWrap::default());
            let top = doc.layers[0].children().map_or(0, Vec::len);
            let id = doc.insert(Some(doc.layers[0].id), top, text).unwrap();
            let look = insert_wrap(&mut doc, Rect::new(40.0, 0.0, 60.0, 100.0), 10_000);
            let look = doc.remove(look).unwrap();
            assert!(resolve_text_wrap(&mut doc).is_ok());
            let warped = ai::WarpedType::unchecked(vec![(id, (*look).clone())]);
            let filled = std::collections::HashMap::from([(id, Filled { text: vec![id], objects: vec![id] })]);
            let mut notes = vec![];
            let settled = settle_warped_type(warped, &mut doc, &filled, &BTreeSet::new(), &BTreeSet::new(), &mut vec![], &mut notes);
            assert_eq!(settled.is_ok(), ok, "{texts} text objects");
            assert_eq!(notes, ["warped text objects that came in as drawn art, not as editable type: 1"]);
        }
    }

    /// Area type that wrapped around an object taken out of the layers flows without it.
    #[test]
    fn wrap_shapes_go_with_their_wrap_object() {
        let mut doc = Document::new(100.0, 100.0);
        let text = insert_area(&mut doc, 0, Rect::new(0.0, 0.0, 100.0, 100.0));
        let wrap = insert_wrap(&mut doc, Rect::new(40.0, 0.0, 60.0, 100.0), 4);
        resolve_text_wrap(&mut doc).unwrap();
        assert_eq!(wrap_shapes(&doc, text), 1);
        doc.remove(wrap).unwrap();
        relayout_wrapped(&mut doc);
        assert_eq!(wrap_shapes(&doc, text), 0);
    }

    /// An image variable binds the linked image its XML id names, and not an embedded image or a
    /// path; a text variable binds the type a slot became, and not a slot that became two text
    /// objects; a visibility variable binds every object that took a slot's place and is still in
    /// the document; a graph variable
    /// binds nothing. A locked object, one in a hidden group and objects on a locked artboard are
    /// bound too. Every binding left out, and those the reader left out, are counted in one note.
    #[test]
    fn variable_bindings_take_objects_of_their_kind() {
        use vectorcraft_doc::{LinkInfo, Variable, Variables};
        let mut doc = Document::new(100.0, 100.0);
        let made = insert_area(&mut doc, 0, Rect::new(0.0, 0.0, 50.0, 50.0));
        let (one, two) = (insert_area(&mut doc, 1, Rect::new(50.0, 0.0, 60.0, 10.0)), insert_area(&mut doc, 2, Rect::new(60.0, 0.0, 70.0, 10.0)));
        let path = insert_wrap(&mut doc, Rect::new(0.0, 60.0, 10.0, 70.0), 4);
        let locked = insert_wrap(&mut doc, Rect::new(20.0, 60.0, 30.0, 70.0), 4);
        let image = |doc: &mut Document, link: Option<LinkInfo>| {
            let id = doc.alloc_id();
            let im = ImageObject { key: "k".into(), width: 1, height: 1, xf: Affine::IDENTITY, link, placement: Default::default(), pixels: None };
            doc.insert(Some(doc.layers[0].id), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
            id
        };
        let (linked, embedded) = (image(&mut doc, Some(LinkInfo::new("/a.png"))), image(&mut doc, None));
        doc.node_mut(locked).unwrap().locked = true;
        let hidden = doc.alloc_id();
        let inside = doc.alloc_id();
        let mut group = Node::group(
            hidden,
            vec![Arc::new(Node::new(
                inside,
                NodeKind::Image(Box::new(ImageObject {
                    key: "k".into(),
                    width: 1,
                    height: 1,
                    xf: Affine::IDENTITY,
                    link: Some(LinkInfo::new("/b.png")),
                    placement: Default::default(),
                    pixels: None,
                })),
            ))],
        );
        group.visible = false;
        doc.insert(Some(doc.layers[0].id), 0, group).unwrap();
        doc.artboards[0].locked = true;
        let var = |name: &str, kind| Variable { name: name.into(), kind };
        doc.variables = Variables {
            variables: vec![
                var("v_image", VariableKind::Image),
                var("v_text", VariableKind::Text),
                var("v_shown", VariableKind::Visibility),
                var("v_graph", VariableKind::Graph),
            ],
            ..Default::default()
        }
        .into();
        // The slots the text objects came from are gone; their ids are the ones the reader gave.
        let (slot, split) = (NodeId(9000), NodeId(9001));
        let ids: std::collections::HashMap<String, Option<NodeId>> = [
            ("linked", Some(linked)),
            ("embedded", Some(embedded)),
            ("path", Some(path)),
            ("locked", Some(locked)),
            ("inside", Some(inside)),
            ("slot", Some(slot)),
            ("split", Some(split)),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into();
        let filled: std::collections::HashMap<NodeId, Filled> = [
            (slot, Filled { text: vec![made], objects: vec![made] }),
            (split, Filled { text: vec![one, two], objects: vec![one, two, NodeId(9002)] }),
        ]
        .into();
        let bind = |variable: &str, target: &str| ai::BindingRef { variable: variable.into(), target: Some(target.into()) };
        let refs = [
            bind("v_image", "linked"),
            bind("v_image", "embedded"),
            bind("v_image", "path"),
            bind("v_image", "inside"),
            bind("v_text", "slot"),
            bind("v_text", "split"),
            bind("v_shown", "split"),
            bind("v_shown", "locked"),
            bind("v_graph", "path"),
        ];
        let mut notes = vec![];
        bind_variables(&mut doc, &refs, 3, &ids, &filled, &mut notes);
        let sorted = |mut v: Vec<NodeId>| {
            v.sort();
            v
        };
        assert_eq!(sorted(doc.variables.objects_of("v_image")), sorted(vec![linked, inside]));
        assert_eq!(doc.variables.objects_of("v_text"), [made]);
        assert_eq!(sorted(doc.variables.objects_of("v_shown")), sorted(vec![one, two, locked]));
        assert!(doc.variables.objects_of("v_graph").is_empty());
        assert_eq!(notes, ai::bindings_note(4 + 3).into_iter().collect::<Vec<_>>());
    }

    /// Area type wrapped around a placed image flows around it at the image's other placement.
    #[test]
    fn wrap_shapes_follow_an_image_to_its_other_placement() {
        let mut doc = Document::new(100.0, 100.0);
        let text = insert_area(&mut doc, 0, Rect::new(0.0, 0.0, 100.0, 100.0));
        let image = doc.alloc_id();
        let mut n = Node::new(
            image,
            NodeKind::Image(Box::new(ImageObject {
                key: "k".into(),
                width: 10,
                height: 10,
                xf: Affine::translate(Vec2::new(10.0, 10.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        );
        n.extras_mut().wrap = Some(vectorcraft_doc::TextWrap::default());
        doc.insert(Some(doc.layers[0].id), 1, n).unwrap();
        resolve_text_wrap(&mut doc).unwrap();
        let x0 = |d: &Document| match d.node(text).map(|n| &n.kind) {
            Some(NodeKind::Text(t)) => t.wrap.first().and_then(|w| w.path.bounds()).map(|b| b.x0),
            _ => None,
        };
        assert_eq!(x0(&doc), Some(10.0));
        let moved = elsewhere(&doc, &[(image, Affine::translate(Vec2::new(50.0, 0.0)))]).unwrap();
        assert_eq!(x0(&moved), Some(60.0));
    }

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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
        let frame = Frame {
            page: &page,
            page_rect: first,
            rect: first,
            whole: false,
            pages: vec![(first, Affine::IDENTITY), (second, Affine::IDENTITY)],
            more: vec![],
        };
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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false, pages: vec![(rect, Affine::IDENTITY)], more: vec![] };
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
