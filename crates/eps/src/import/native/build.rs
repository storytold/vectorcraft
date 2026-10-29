use super::compare::{MAX_DIFFERENCE, PageView, compare};
use super::plan::plan;
use super::slots::{
    ART_LEFT_OUT, Content, MAX_PAIRS, MAX_SLOTS, MAX_TEXT, OUTLINE_REACH, TEXT_LEFT_OUT, TYPE_LEFT_OUT, collect_slots, extent, few, fill_slots,
    text_objects,
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
    let Structure { mut doc, to_doc, bbox, artboard, template, hidden_unread, slot_names, restyled, warnings } = structure;
    // Editing data in the units of the page's user space ([`in_units`]) is compared with the page
    // scaled down to those units, and the document read from it is scaled up by `unit`.
    let board = artboard.map(|a| to_doc.transform_rect_bbox(a));
    let small = visible.filter(|v| matches!(page, Page::Artboard) && in_units(board, v, unit)).map(|v| {
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
        let rect = to_doc.transform_rect_bbox(art_box);
        if (rect.width() - page_rect.width()).abs() > 0.5 || (rect.height() - page_rect.height()).abs() > 0.5 {
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
        let same =
            |(a, p): (&vectorcraft_doc::Artboard, &Rect)| (a.rect.width() - p.width()).abs() <= 0.5 && (a.rect.height() - p.height()).abs() <= 0.5;
        let boards = (pages.len() == doc.artboards.len() && doc.artboards.iter().zip(&pages).all(same)).then_some(&doc.artboards);
        for object in shown.iter_mut().chain(hidden.iter_mut()).chain(lone.iter_mut()) {
            // The page with most of it, the first for what is on none.
            let on = extent(object).and_then(|r| {
                let inside = |p: &Rect| p.intersect(r).area().max(0.0);
                pages.iter().enumerate().skip(1).filter(|(_, p)| inside(p) > inside(&page_rect)).max_by(|a, b| inside(a.1).total_cmp(&inside(b.1)))
            });
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
        }
        frame = Some(Frame { page: visible, page_rect, rect, whole });
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
    if infos.iter().all(|i| !i.shown) && !shown.is_empty() {
        return Err("its page has text, and its layers have no text objects to put it in".into());
    }
    // The page's type is compared with every text object of the layers: that has its limits.
    if infos.len() > MAX_SLOTS || infos.len().saturating_mul(shown.len()) > MAX_PAIRS {
        return Err("it has too many text objects".into());
    }
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
    let content = plan(&infos, &stories, template, to_doc, [&shown, &hidden], &mut notes);
    // The glyphs left out with the rest of the page's art, and how many text objects take type of
    // the page right next to them (not counted past `MAX_PAIRS` comparisons).
    let compared = lone.len().saturating_mul(shown.iter().chain(&hidden).map(Vec::len).sum());
    let missing: Vec<Rect> = if compared > MAX_PAIRS { vec![] } else { lone.iter().filter_map(|g| extent(g)).collect() };
    let next_to_missing = |n: &Arc<Node>| {
        matches!(n.kind, NodeKind::Text(_))
            && n.visual_bounds().is_some_and(|b| missing.iter().any(|r| !b.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(*r).is_zero_area()))
    };
    let short = content.iter().filter(|c| matches!(c, Content::Page(p) | Content::Made(_, _, _, p) if p.iter().any(next_to_missing))).count();
    let mut empty = 0;
    let mut layers = std::mem::take(&mut doc.layers);
    let mut made = vec![];
    fill_slots(&mut doc, &mut layers, &mut content.into_iter(), &slot_names, &mut empty, &mut made, &mut vec![]);
    doc.layers = layers;
    // A story in several frames is threaded type: its frames in order (the first holds the story).
    made.sort();
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
    if !hidden_unread.is_empty() {
        notes.push(format!("{ART_LEFT_OUT} ({}), left out of them", few(hidden_unread)));
    }
    match frame {
        Some(frame) => {
            let mut view = PageView::new(&frame);
            view.restyled(&restyled, frame.rect);
            let unseen = prune_unseen(&frame, &mut view, &mut doc);
            if unseen > 0 {
                notes.push(format!("{unseen} {UNSEEN_TEXT}"));
            }
            match compare(&frame, &mut view, &doc) {
                Ok(note) => notes.extend(note),
                Err(why) if trust => notes.push(format!("{why}: they come from its editing data all the same")),
                Err(why) => return Err(why),
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

/// Does the editing data's first artboard `board` (in the document read from it: art space through
/// `to_doc`) have the size of `page`'s first artboard in units of `unit` points, `unit` being above
/// 1, and not its size in points? `page` is a PDF part read in points from pages of `unit` points a
/// unit (their `UserUnit`).
fn in_units(board: Option<Rect>, page: &Document, unit: f64) -> bool {
    let (Some(board), Some(p)) = (board, page.artboards.first().map(|a| a.rect)) else { return false };
    let fits = |k: f64| (board.width() - p.width() / k).abs() <= 0.5 && (board.height() - p.height() / k).abs() <= 0.5;
    unit.is_finite() && unit > 1.0 && fits(unit) && !fits(1.0)
}

/// Draw `doc`'s layers and artboards `k` times as large, from the origin, with their strokes,
/// effects and pattern tiles.
fn scale_document(doc: &mut Document, k: f64) {
    let a = Affine::scale(k);
    let scaling = Scaling { strokes: true, effects: Some(vectorcraft_effects::scale_effect), patterns: true, ..Default::default() };
    for layer in &mut doc.layers {
        Arc::make_mut(layer).transform(a, scaling);
    }
    for board in &mut doc.artboards {
        board.rect = a.transform_rect_bbox(board.rect);
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
/// of what was turned to outlines): they are taken out. How many.
pub(super) fn prune_unseen(frame: &Frame<'_>, view: &mut PageView, doc: &mut Document) -> usize {
    let mut found = vec![];
    unmark(&mut doc.layers, true, true, &mut found);
    let rect = frame.rect;
    if found.is_empty() || found.len() > MAX_WEIGHED {
        return 0;
    }
    let with = view.difference(doc, rect).any;
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
