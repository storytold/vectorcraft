//! The layers of an Illustrator EPS or `.ai`.
//!
//! Besides the page it prints, such a file carries the app's own copy of the art: an EPS after its
//! `%%EOF`, a `.ai` in its `AIPrivateData` streams (see `ai` for the containers). It has what the
//! page doesn't: the layers with their names and options, the groups, compound paths and clipping
//! groups, the objects' names, the hidden objects and layers, the art outside the artboards and
//! every artboard. `ai` reads that structure into a document, with the colours, gradients,
//! transparency and images of the art.
//!
//! Text objects are slots in that document: their characters, fonts and places are in the file's
//! text document (see `ate`). Type that shows comes from the page, which draws it, into the slot
//! whose text is where the page has it. Type that doesn't show (a hidden layer or object, or off
//! the page) is made from the text document; where that can't be read (area type, type on a path)
//! it's left out, with a warning.
//!
//! What a layer that shows has that `ai` doesn't read makes the file come in as its page, as it did
//! before; on a layer that doesn't show it is left out of that layer, with a warning. The page is
//! also the safety net: when the layers' art looks different from it (one draws an object the
//! other doesn't, or much of it differs; an EPS whose page is its art's box has nothing printed
//! outside it), the page is used.
//!
//! Where a file doesn't match what this module expects, it is left alone and its page is imported.

use std::collections::BTreeSet;
use std::sync::Arc;

use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::Imported;
use super::ai::{self, Structure, slot_of};
use super::ate::{Story, Texts};

/// The notes that say the import left out something the file has (see [`is_loss`]).
const TEXT_LEFT_OUT: &str = "text objects on hidden layers or hidden objects couldn't be read, so they are left out";
/// The same for a file without a page to take shown type from.
const TYPE_LEFT_OUT: &str = "text objects of kinds VectorCraft doesn't read from the file's text document yet are left out";
const ART_LEFT_OUT: &str = "hidden layers have art this can't read";
const LAYERS_UNREAD: &str = "the file's layers weren't read";

/// Does this import note say that the file has something the document doesn't (hidden text, art or
/// layers that could not be read)? Writing the document over the file would lose it for good.
pub fn is_loss(note: &str) -> bool {
    note.ends_with(TEXT_LEFT_OUT)
        || note.ends_with(TYPE_LEFT_OUT)
        || [ART_LEFT_OUT, LAYERS_UNREAD, super::graphics::TOO_MUCH, super::graphics::FAR_AWAY].iter().any(|n| note.starts_with(n))
}

/// Most text objects in the layers, and most pieces of type in one place on the page, that are matched
/// to each other (more than this and the layers aren't used).
const MAX_SLOTS: usize = 5000;
const MAX_PIECES: usize = 2000;
/// Most bytes of text in the text objects of the layers (a story named twice counts twice).
const MAX_TEXT: usize = 16 << 20;
/// Most pairs of a text object and a piece of the page's type compared.
const MAX_PAIRS: usize = 4_000_000;

/// `names` for a note: the first few, and how many more.
fn few(names: BTreeSet<String>) -> String {
    const SHOWN: usize = 5;
    let more = names.len().saturating_sub(SHOWN);
    let mut list: Vec<String> = names.into_iter().take(SHOWN).collect();
    if more > 0 {
        list.push(format!("{more} more"));
    }
    list.join(", ")
}

/// A text slot of the layers.
struct SlotInfo {
    story: Option<u32>,
    /// Which of the story's frames it is.
    frame: usize,
    /// It, its layer and the objects round it show.
    shown: bool,
}

/// The text slots of the layers and objects, in order.
fn collect_slots(nodes: &[Arc<Node>], shown: bool, out: &mut Vec<SlotInfo>) {
    for n in nodes {
        let shown = shown && n.visible;
        if let Some(story) = slot_of(n.name.as_deref()) {
            out.push(SlotInfo { story, frame: ai::slot_frame(n.name.as_deref()), shown });
        } else if let Some(c) = n.children().filter(|_| !matches!(n.kind, NodeKind::Compound { .. })) {
            collect_slots(c, shown, out);
        }
    }
}

/// How far from a text object's own extent the strokes drawn around it reach (points).
const OUTLINE_REACH: f64 = 6.0;

/// Does `n` look like part of a text object's outline: a path with a stroke, inside `around`?
fn outlines(n: &Node, around: Rect) -> bool {
    matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
        && n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.visible && s.width > 0.0))
        && n.visual_bounds().is_some_and(|b| {
            let a = around.inflate(OUTLINE_REACH, OUTLINE_REACH);
            b.x0 >= a.x0 && b.y0 >= a.y0 && b.x1 <= a.x1 && b.y1 <= a.y1
        })
}

/// How far apart (points) the baselines of two text nodes may be for them to be on one line.
const SAME_LINE: f64 = 0.5;

/// Where `n`'s baseline lies across the direction it runs in (points), if it is text.
fn baseline(n: &Node) -> Option<f64> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let [a, b, _, _, e, f] = t.xf.as_coeffs();
    let len = a.hypot(b);
    (len > 1e-9).then(|| (a * f - b * e) / len)
}

/// The text objects the page paints, each as the run of nodes that make it up (its text, once for
/// each fill and stroke it has, with the paths of its strokes), in painting order: those that
/// show in `shown`, those of hidden layers and objects (a PDF has them) in `hidden`.
fn text_objects(nodes: &[Arc<Node>], on: bool, shown: &mut Vec<Vec<Arc<Node>>>, hidden: &mut Vec<Vec<Arc<Node>>>) {
    let is_text = |n: &Node| matches!(n.kind, NodeKind::Text(_));
    let (mut i, mut free) = (0, 0);
    while let Some(n) = nodes.get(i) {
        if !is_text(n) {
            if let Some(c) = n.children().filter(|_| !matches!(n.kind, NodeKind::Compound { .. })) {
                text_objects(c, on && n.visible, shown, hidden);
                free = i + 1;
            }
            i += 1;
            continue;
        }
        let mut bounds = n.visual_bounds().unwrap_or_default();
        let (mut start, mut end) = (i, i);
        while start > free
            && let Some(p) = nodes.get(start - 1)
            && outlines(p, bounds)
        {
            start -= 1;
            bounds = bounds.union(p.visual_bounds().unwrap_or_default());
        }
        while let Some(next) = nodes.get(end + 1)
            && let Some(b) = next.visual_bounds()
            && (is_text(next)
                && !b.intersect(bounds.inflate(OUTLINE_REACH, OUTLINE_REACH)).is_zero_area()
                && baseline(next)
                    .zip(nodes.get(start..=end).unwrap_or_default().iter().find_map(|m| baseline(m)))
                    .is_some_and(|(x, y)| (x - y).abs() <= SAME_LINE)
                || outlines(next, bounds))
        {
            end += 1;
            bounds = bounds.union(b);
        }
        let object = nodes.get(start..=end).unwrap_or_default().to_vec();
        let into = if on && n.visible { &mut *shown } else { &mut *hidden };
        into.push(object);
        i = end + 1;
        free = i;
    }
}

/// `n` and what it holds with ids of `doc`.
fn reid(doc: &mut Document, n: &Arc<Node>) -> Arc<Node> {
    let mut n = n.as_ref().clone();
    n.id = doc.alloc_id();
    if let Some(children) = n.children_mut() {
        for c in children.iter_mut() {
            *c = reid(doc, c);
        }
    }
    Arc::new(n)
}

/// The page's text objects given to each of `slots` text slots: in painting order, and when there
/// are more of one than of the other (a label may be a few text objects, or one with several
/// strokes), each goes where the share of the order it is at falls.
fn assign(objects: &[Vec<Arc<Node>>], slots: usize) -> Vec<Vec<&[Arc<Node>]>> {
    let mut given: Vec<Vec<&[Arc<Node>]>> = vec![vec![]; slots];
    for (j, o) in objects.iter().enumerate() {
        if let Some(at) = given.get_mut(((2 * j + 1) * slots / (2 * objects.len())).min(slots.saturating_sub(1))) {
            at.push(o);
        }
    }
    given
}

/// What goes into a text slot.
enum Content {
    /// Text objects (with the strokes round them) of the page.
    Page(Vec<Arc<Node>>),
    /// A text object made from the file's text document, where it lies on the document, whether
    /// the page has type where it is, and the page's type of no text object that is nearest it
    /// (kept after it).
    Made(Box<Node>, Option<Rect>, bool, Vec<Arc<Node>>),
    /// Nothing the file lets this read.
    Empty,
    /// Its type is on the page with another text object's; there is nothing to put here.
    Merged,
}

/// Replace the text slots in `nodes` with what `content` has for each, in order (a slot given none is
/// left out): the page's type takes its place as it is, made type keeps the slot's name. `empty`
/// counts those left out.
fn fill_slots(
    doc: &mut Document,
    nodes: &mut Vec<Arc<Node>>,
    content: &mut std::vec::IntoIter<Content>,
    names: &std::collections::HashMap<NodeId, String>,
    empty: &mut usize,
    made: &mut Vec<(u32, usize, NodeId)>,
) {
    let mut i = 0;
    while i < nodes.len() {
        let Some(n) = nodes.get(i) else { break };
        if let Some(story) = slot_of(n.name.as_deref()) {
            let visible = n.visible;
            let name = names.get(&n.id);
            let frame = ai::slot_frame(n.name.as_deref());
            let replacement = match content.next() {
                Some(Content::Page(parts)) if !parts.is_empty() => {
                    // The page's type (and the strokes round it) take the slot's place as they are.
                    let parts: Vec<Arc<Node>> = parts
                        .iter()
                        .map(|p| {
                            let mut p = reid(doc, p);
                            if !visible {
                                Arc::make_mut(&mut p).visible = false;
                            }
                            if let Some(name) = name.filter(|_| matches!(p.kind, NodeKind::Text(_))) {
                                Arc::make_mut(&mut p).name = Some(name.clone());
                            }
                            p
                        })
                        .collect();
                    let n = parts.len();
                    nodes.splice(i..=i, parts);
                    i += n;
                    continue;
                }
                Some(Content::Made(mut text, _, seen, extra)) => {
                    text.id = doc.alloc_id();
                    if let Some(story) = story {
                        made.push((story, frame, text.id));
                    }
                    text.visible = visible;
                    // Type the page shows is kept; other made type is weighed against the page.
                    text.name = if seen { name.cloned() } else { Some(format!("{MADE}{}", name.map_or("", String::as_str))) };
                    let mut parts = vec![Arc::new(*text)];
                    parts.extend(extra.iter().map(|p| reid(doc, p)));
                    let n = parts.len();
                    nodes.splice(i..=i, parts);
                    i += n;
                    continue;
                }
                Some(Content::Merged) => {
                    nodes.remove(i);
                    continue;
                }
                _ => None,
            };
            match replacement {
                Some(node) => {
                    if let Some(slot) = nodes.get_mut(i) {
                        *slot = Arc::new(node);
                    }
                    i += 1;
                }
                None => {
                    *empty += 1;
                    nodes.remove(i);
                }
            }
        } else {
            if n.children().is_some()
                && !matches!(n.kind, NodeKind::Compound { .. })
                && let Some(c) = nodes.get_mut(i).map(Arc::make_mut).and_then(Node::children_mut)
            {
                fill_slots(doc, c, content, names, empty, made);
            }
            i += 1;
        }
    }
}

/// How far, in points, the page's type may be from where the file's text document puts it for the
/// two to be the same type.
const SAME_PLACE: f64 = 0.75;

/// Its letters and digits, which is what is compared of type (the page may spell a mark another way).
fn letters(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// Does `n` hold type that is where `story` lays one of its lines out: the line, or a piece of it
/// (the page splits a line where its kerning or its styles change), on that line's baseline and
/// after its start? How far from where the line starts it is, if so.
fn same_type(n: &Node, lines: &[(String, Point)]) -> Option<f64> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let have = letters(&t.plain_text());
    let [a, b, _, _, e, f] = t.xf.as_coeffs();
    let len = a.hypot(b);
    if have.is_empty() || len < 1e-9 {
        return None;
    }
    let (ux, uy) = (a / len, b / len);
    // About how far a character of it advances, to say how far along a line a piece may be.
    let advance = n.visual_bounds().map_or(0.0, |r| r.width().max(r.height())) / have.len() as f64;
    lines
        .iter()
        .filter_map(|(text, at)| {
            let line = letters(text);
            let skipped = line.find(&have)?;
            if skipped == 0 {
                return Some(((at.x - e).powi(2) + (at.y - f).powi(2)).sqrt()).filter(|d| *d <= SAME_PLACE);
            }
            let (dx, dy) = (e - at.x, f - at.y);
            let (along, across) = (dx * ux + dy * uy, (dx * uy - dy * ux).abs());
            ((have.len() > 1 || skipped + 1 == line.len())
                && across <= SAME_PLACE
                && along >= -SAME_PLACE
                && along <= line.len() as f64 * advance * 2.0 + 4.0)
                .then_some(across + 0.001 * along)
        })
        .min_by(f64::total_cmp)
}

/// The page draws a line in pieces where its kerning or styles change ("swe", "ep"), and the file
/// has it as one line of one text object: the pieces that on one baseline add up to a line of
/// `lines` become one text (each keeping its own style, the line's spaces between them).
fn join_pieces(parts: &mut Vec<Arc<Node>>, lines: &[(String, Point)]) {
    let text_of = |n: &Arc<Node>| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None };
    let texts: Vec<usize> = parts.iter().enumerate().filter(|(_, n)| matches!(n.kind, NodeKind::Text(_))).map(|(i, _)| i).collect();
    // A page with this many pieces of type in one place isn't one of lines of a story.
    if texts.len() > MAX_PIECES {
        return;
    }
    let mut taken = vec![false; parts.len()];
    // The first piece of each line to join, the others, and the spaces to put after each piece.
    let mut joins: Vec<(usize, Vec<usize>, Vec<String>)> = vec![];
    for &i in &texts {
        if taken.get(i).copied().unwrap_or(true) {
            continue;
        }
        let Some(line) = parts.get(i).and_then(|n| baseline(n)) else { continue };
        let group: Vec<usize> = texts
            .iter()
            .copied()
            .filter(|j| {
                !taken.get(*j).copied().unwrap_or(true) && parts.get(*j).and_then(|n| baseline(n)).is_some_and(|b| (b - line).abs() <= SAME_LINE)
            })
            .collect();
        for j in &group {
            if let Some(t) = taken.get_mut(*j) {
                *t = true;
            }
        }
        let pieces: Vec<String> = group.iter().filter_map(|j| parts.get(*j).and_then(text_of)).collect();
        let places: Vec<(f64, f64)> = group
            .iter()
            .filter_map(|j| parts.get(*j))
            .filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some((t.xf.as_coeffs()[4], t.xf.as_coeffs()[5])) } else { None })
            .collect();
        let distinct = places.iter().enumerate().all(|(a, p)| places.iter().skip(a + 1).all(|q| (p.0 - q.0).hypot(p.1 - q.1) > 0.01));
        let joined: String = pieces.iter().map(|p| letters(p)).collect();
        if group.len() < 2 || !distinct || joined.is_empty() {
            continue;
        }
        let Some((whole, _)) = lines.iter().find(|(text, _)| letters(text) == joined) else { continue };
        // Where the line has spaces after a piece's last character that the page doesn't draw.
        let line_chars: Vec<char> = whole.chars().collect();
        let mut at = 0;
        let mut gaps = vec![];
        for (k, piece) in pieces.iter().enumerate() {
            for _ in piece.chars().filter(|c| !c.is_whitespace()) {
                while line_chars.get(at).is_some_and(|c| c.is_whitespace()) {
                    at += 1;
                }
                at += 1;
            }
            let space: String = line_chars.iter().skip(at).take_while(|c| c.is_whitespace()).collect();
            let next = pieces.get(k + 1);
            let drawn = piece.ends_with(char::is_whitespace) || next.is_none_or(|n| n.starts_with(char::is_whitespace));
            gaps.push(if drawn { String::new() } else { space });
        }
        if at <= line_chars.len() {
            joins.push((group[0], group[1..].to_vec(), gaps));
        }
    }
    let mut gone: BTreeSet<usize> = BTreeSet::new();
    for (first, rest, gaps) in &joins {
        let mut runs: Vec<Vec<vectorcraft_doc::TextRun>> = [first]
            .into_iter()
            .chain(rest)
            .filter_map(|j| parts.get(*j))
            .filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.runs.clone()) } else { None })
            .collect();
        for (r, gap) in runs.iter_mut().zip(gaps) {
            if let Some(last) = r.last_mut() {
                last.text.push_str(gap);
            }
        }
        if let Some(node) = parts.get_mut(*first).map(Arc::make_mut)
            && let NodeKind::Text(t) = &mut node.kind
        {
            t.runs = runs.into_iter().flatten().collect();
            // The layout cached for the first piece is not this text's.
            t.cached_bounds = None;
            t.cached_baselines.clear();
        }
        gone.extend(rest);
    }
    // The pieces that went into a text, removed once the joins are done: removing as each is made
    // would move the places of the pieces that the others still name.
    let mut keep = 0;
    parts.retain(|_| {
        keep += 1;
        !gone.contains(&(keep - 1))
    });
}

/// Do the regions of made type cover most of `object`?
fn covered(object: &[Arc<Node>], content: &[Content]) -> bool {
    let Some(r) = extent(object) else { return false };
    let inside: f64 =
        content.iter().filter_map(|c| if let Content::Made(_, Some(region), ..) = c { Some(region.intersect(r).area()) } else { None }).sum();
    inside >= 0.5 * r.area().max(1e-9)
}

/// The box round `parts`.
fn extent(parts: &[Arc<Node>]) -> Option<Rect> {
    parts.iter().filter_map(|n| n.visual_bounds()).reduce(|a, b| a.union(b))
}

/// What goes into each of `infos`' slots: the page's type that the file's text document puts where the
/// page has it, else type made from the text document, else (for a file whose text document can't say)
/// the page's type in painting order.
fn plan(
    infos: &[SlotInfo],
    stories: &[Option<Arc<Story>>],
    template: Option<(f64, f64)>,
    to_doc: Affine,
    pages: [&[Vec<Arc<Node>>]; 2],
    warn: &mut Vec<String>,
) -> Vec<Content> {
    let [shown, hidden] = pages;
    let lines: Vec<Option<Vec<(String, Point)>>> = stories
        .iter()
        .map(|s| {
            let (s, t) = (s.as_ref()?, template?);
            Some(s.line_starts(t).into_iter().map(|(text, (x, y))| (text, to_doc * Point::new(x, y))).collect())
        })
        .collect();
    // The page's objects that are where a shown slot's lines are, nearest first.
    let mut pairs: Vec<(f64, usize, usize)> = vec![];
    for (k, info) in infos.iter().enumerate() {
        let Some(l) = lines.get(k).and_then(Option::as_ref).filter(|_| info.shown) else { continue };
        for (j, object) in shown.iter().enumerate() {
            if let Some(d) = object.iter().filter_map(|n| same_type(n, l)).min_by(f64::total_cmp) {
                pairs.push((d, k, j));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut owner: Vec<Option<usize>> = vec![None; shown.len()];
    for (_, k, j) in pairs {
        if let Some(o) = owner.get_mut(j).filter(|o| o.is_none()) {
            *o = Some(k);
        }
    }
    // Type the text document makes, shown or not: the page's pieces of it aren't needed.
    let mut content: Vec<Content> = infos.iter().map(|_| Content::Empty).collect();
    if let Some(t) = template {
        for (k, info) in infos.iter().enumerate() {
            let Some(Some(story)) = stories.get(k) else { continue };
            let Some(node) = story.node(NodeId(0), info.frame, t, to_doc) else { continue };
            let region = story.region(t, to_doc);
            if let Some(c) = content.get_mut(k) {
                *c = Content::Made(Box::new(node), region.filter(|_| info.shown), false, vec![]);
            }
        }
    }
    // The file's text document is trusted where it agrees with the page about something (its lines
    // start where the page's type does, or the page has type where it lays type out), or where the
    // page has no type to disagree with.
    let in_region = |object: &[Arc<Node>]| covered(object, &content);
    let trusted = owner.iter().any(Option::is_some) || shown.is_empty() || shown.iter().any(|o| in_region(o));
    if !trusted {
        content.iter_mut().for_each(|c| *c = Content::Empty);
    }
    let mut spare: Vec<Vec<Arc<Node>>> = vec![];
    for (j, object) in shown.iter().enumerate() {
        match owner.get(j).copied().flatten().filter(|_| trusted) {
            Some(k) => match content.get_mut(k) {
                Some(Content::Made(_, _, seen, _)) => *seen = true,
                Some(Content::Page(p)) => p.extend(object.iter().cloned()),
                Some(c) => *c = Content::Page(object.clone()),
                None => {}
            },
            // What lies where made type is, is that type as the page draws it.
            None if covered(object, &content) => {
                let r = extent(object).unwrap_or(Rect::ZERO);
                for c in &mut content {
                    if let Content::Made(_, Some(region), seen, _) = c
                        && !region.intersect(r).is_zero_area()
                    {
                        *seen = true;
                    }
                }
            }
            None => spare.push(object.clone()),
        }
    }
    for (k, c) in content.iter_mut().enumerate() {
        if let (Content::Page(parts), Some(Some(l))) = (c, lines.get(k)) {
            join_pieces(parts, l);
        }
    }
    // What is left of the page goes by painting order into the shown slots the file can't say.
    let loose: Vec<usize> =
        infos.iter().enumerate().filter(|(k, i)| i.shown && matches!(content.get(*k), Some(Content::Empty))).map(|(k, _)| k).collect();
    if loose.is_empty() {
        // Type of the page that no text object says it owns goes with the nearest text object that has
        // type of the page; with none, it is left out.
        let mut left_out = false;
        for object in &spare {
            let (centre, near) = (extent(object).map(|r| r.center()), |c: &Content| match c {
                Content::Page(parts) => extent(parts).map(|r| r.center()),
                Content::Made(_, region, ..) => region.map(|r| r.center()),
                _ => None,
            });
            let nearest = content
                .iter_mut()
                .filter_map(|c| near(c).zip(centre).map(|(p, q)| ((p.x - q.x).hypot(p.y - q.y), c)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            match nearest {
                Some((_, Content::Page(parts) | Content::Made(_, _, _, parts))) => parts.extend(object.iter().cloned()),
                _ => left_out = true,
            }
        }
        if !spare.is_empty() {
            warn.push(
                if left_out {
                    "some of the page's type has no text object in the layers, so it is left out"
                } else {
                    "some of the page's type has no text object of its own in the layers, so it is with another's"
                }
                .into(),
            );
        }
    } else {
        for (slot, parts) in loose.iter().zip(assign(&spare, loose.len())) {
            if let Some(c) = content.get_mut(*slot) {
                // A text object the page shows as part of another's has nothing to hold of its own.
                *c = if parts.is_empty() && !spare.is_empty() {
                    Content::Merged
                } else {
                    Content::Page(parts.into_iter().flat_map(|p| p.iter().cloned()).collect())
                };
            }
        }
        if !spare.is_empty() && spare.len() != loose.len() {
            warn.push("the text is on the layers in the order the page paints it, so a text object may be in another group than it was".into());
        }
    }
    // The same for the slots that don't show.
    let unread: Vec<usize> =
        infos.iter().enumerate().filter(|(k, i)| !i.shown && matches!(content.get(*k), Some(Content::Empty))).map(|(k, _)| k).collect();
    for (slot, parts) in unread.iter().zip(assign(hidden, unread.len())) {
        if let Some(c) = content.get_mut(*slot) {
            *c = Content::Page(parts.into_iter().flat_map(|p| p.iter().cloned()).collect());
        }
    }
    content
}

/// `doc` with its layers that don't print hidden, when it has any.
fn as_printed(doc: &Document) -> Option<Document> {
    fn hide(nodes: &mut [Arc<Node>]) -> bool {
        let mut any = false;
        for n in nodes {
            if matches!(n.kind, NodeKind::Layer { printable: false, .. }) && n.visible {
                Arc::make_mut(n).visible = false;
                any = true;
            } else if matches!(n.kind, NodeKind::Layer { .. })
                && let Some(c) = Arc::make_mut(n).children_mut()
            {
                any |= hide(c);
            }
        }
        any
    }
    let mut d = doc.clone();
    hide(&mut d.layers).then_some(d)
}

/// A premultiplied pixel seen on white.
fn on_white(p: &[u8; 4]) -> [u32; 3] {
    let paper = 255 - u32::from(p[3]).min(255);
    [0, 1, 2].map(|i| (u32::from(p[i]) + paper).min(255))
}

/// The lightness (0 to 255, Rec. 601 weights) of a pixel seen on white.
fn lightness([r, g, b]: [u32; 3]) -> u32 {
    (r * 299 + g * 587 + b * 114) / 1000
}

/// How far (0 to 255) a pixel seen on white is from white: how much ink it has.
fn ink([r, g, b]: [u32; 3]) -> u32 {
    255 - r.min(g).min(b)
}

/// How far apart two pixels' lightness, or their ink where one is white, may be for them to look
/// alike.
const SAME_PIXEL: u32 = 48;
/// The most ink a pixel seen as white has.
const WHITE: u32 = 8;

/// Do two pixels look different: in lightness, or one inked (as a light yellow is) where the other
/// is white?
fn differ(p: &[u8; 4], q: &[u8; 4]) -> bool {
    let (p, q) = (on_white(p), on_white(q));
    let (a, b) = (ink(p), ink(q));
    lightness(p).abs_diff(lightness(q)) > SAME_PIXEL || (a.min(b) <= WHITE && a.abs_diff(b) > SAME_PIXEL)
}

/// How much two documents draw differently: the share (0 to 1) of the pixels that differ, and of
/// those whose neighbours all differ too, away from type (where one draws an object that the other
/// doesn't, not the edges of one drawn a little differently, or type laid out a little
/// differently: a line the page sets in pieces is laid out afresh as one).
#[derive(Clone, Copy)]
struct Difference {
    any: f64,
    solid: f64,
}

/// The scale the comparisons draw a region at: about 400 pixels along its longer side.
fn compare_scale(r: Rect) -> f64 {
    (400.0 / r.width().max(r.height()).max(1.0)).min(4.0)
}

/// The page drawn once, to weigh the layers' art against it: [`Self::difference`] draws the layers
/// with a renderer of their own, which keeps their decoded images between draws (the page's images
/// can share their keys and not their pixels, so the two don't share one).
struct PageView {
    page: vectorcraft_render::Rendered,
    /// Where the page's own type is, in its pixels.
    page_type: Vec<Rect>,
    layers: vectorcraft_render::Renderer,
    scale: f64,
}

impl PageView {
    fn new(frame: &Frame<'_>) -> Self {
        let ra = frame.page_rect;
        let scale = compare_scale(ra);
        // Drawn on nothing and then seen on white: blending modes have no page to blend with, as in
        // the app that wrote the file.
        let page = vectorcraft_render::Renderer::new().render_region(frame.page, ra, scale, false);
        let mut page_type = vec![];
        type_areas(&frame.page.layers, ra, scale, &mut page_type);
        Self { page, page_type, layers: vectorcraft_render::Renderer::new(), scale }
    }

    /// How `b` on `rb` (a rectangle the page's size) draws differently from the page ([`compared`]).
    fn difference(&mut self, b: &Document, rb: Rect) -> Difference {
        // The page doesn't print what isn't printed.
        let printed = as_printed(b);
        let b = printed.as_ref().unwrap_or(b);
        let y = self.layers.render_region(b, rb, self.scale, false);
        let mut areas = self.page_type.clone();
        type_areas(&b.layers, rb, self.scale, &mut areas);
        compared(&self.page, &y, &areas)
    }
}

/// How `a` on `ra` and `b` on `rb` (rectangles of the same size) draw differently ([`compared`]),
/// for a region other than the page's ([`PageView`] draws that one once).
fn difference(a: &Document, ra: Rect, b: &Document, rb: Rect) -> Difference {
    let printed = as_printed(b);
    let b = printed.as_ref().unwrap_or(b);
    let scale = compare_scale(ra);
    // A renderer each: the two documents' images can share their keys and not their pixels.
    let x = vectorcraft_render::Renderer::new().render_region(a, ra, scale, false);
    let y = vectorcraft_render::Renderer::new().render_region(b, rb, scale, false);
    let mut areas = vec![];
    type_areas(&a.layers, ra, scale, &mut areas);
    type_areas(&b.layers, rb, scale, &mut areas);
    compared(&x, &y, &areas)
}

/// How two drawings of the same size differ, each drawn on nothing and seen on white, away from
/// the type in `areas` (pixels). Pixels are compared by lightness, and by whether they are inked
/// at all ([`differ`]): the page of an RGB document is written in the colours' CMYK equivalents,
/// and what the comparison looks for is art that is missing or out of place.
fn compared(x: &vectorcraft_render::Rendered, y: &vectorcraft_render::Rendered, areas: &[Rect]) -> Difference {
    if x.width != y.width || x.height != y.height {
        return Difference { any: 1.0, solid: 1.0 };
    }
    let differs: Vec<bool> = x.pixels.as_chunks::<4>().0.iter().zip(y.pixels.as_chunks::<4>().0).map(|(p, q)| differ(p, q)).collect();
    let (w, h) = (x.width as usize, x.height as usize);
    let at = |i: usize, j: usize| j.checked_mul(w).and_then(|row| row.checked_add(i)).and_then(|k| differs.get(k)).copied().unwrap_or(false);
    let typed = in_areas(areas, w, h);
    // A pixel on the region's edge has neighbours outside it, which don't count as differing.
    let solid = (1..h.saturating_sub(1))
        .flat_map(|j| (1..w.saturating_sub(1)).map(move |i| (i, j)))
        .filter(|&(i, j)| !typed.get(j * w + i).copied().unwrap_or(true) && (j - 1..=j + 1).all(|n| (i - 1..=i + 1).all(|m| at(m, n))))
        .count();
    let area = (w as f64 * h as f64).max(1.0);
    Difference { any: differs.iter().filter(|d| **d).count() as f64 / area, solid: solid as f64 / area }
}

/// Where the type that shows in `nodes` is, in pixels of `region` drawn at `scale` (with a margin
/// for the strokes round it).
fn type_areas(nodes: &[Arc<Node>], region: Rect, scale: f64, out: &mut Vec<Rect>) {
    for n in nodes.iter().filter(|n| n.visible) {
        if let NodeKind::Text(_) = n.kind
            && let Some(b) = n.visual_bounds()
        {
            let px =
                |r: Rect| Rect::new((r.x0 - region.x0) * scale, (r.y0 - region.y0) * scale, (r.x1 - region.x0) * scale, (r.y1 - region.y0) * scale);
            out.push(px(b.inflate(OUTLINE_REACH, OUTLINE_REACH)));
        } else if let Some(c) = n.children() {
            type_areas(c, region, scale, out);
        }
    }
}

/// Which of `w` × `h` pixels (row by row) lie in one of `areas`.
fn in_areas(areas: &[Rect], w: usize, h: usize) -> Vec<bool> {
    // Each area adds one inside it: corners marked, then summed along the rows and down the columns.
    let stride = w + 1;
    let mut count = vec![0i64; stride * (h + 1)];
    let clamp = |v: f64, max: usize| if v.is_finite() { v.clamp(0.0, max as f64) as usize } else { 0 };
    for r in areas {
        let (x0, y0, x1, y1) = (clamp(r.x0.floor(), w), clamp(r.y0.floor(), h), clamp(r.x1.ceil(), w), clamp(r.y1.ceil(), h));
        for (x, y, d) in [(x0, y0, 1), (x1, y0, -1), (x0, y1, -1), (x1, y1, 1)] {
            if let Some(c) = count.get_mut(y * stride + x) {
                *c += d;
            }
        }
    }
    for row in count.chunks_mut(stride) {
        let mut run = 0;
        for c in row {
            run += *c;
            *c = run;
        }
    }
    let mut column = vec![0i64; stride];
    for row in count.chunks_mut(stride) {
        for (c, sum) in row.iter_mut().zip(column.iter_mut()) {
            *sum += *c;
            *c = *sum;
        }
    }
    (0..h).flat_map(|j| (0..w).map(move |i| (i, j))).map(|(i, j)| count.get(j * stride + i).is_some_and(|c| *c > 0)).collect()
}

/// More of the page than this differing between the layers' art and the page's own, and the
/// layers aren't used.
const MAX_DIFFERENCE: f64 = 0.05;
/// More than this and the import says so.
const NOTABLE_DIFFERENCE: f64 = 0.002;
/// More of the page than this where one draws an object that the other doesn't (see
/// [`Difference`]), and the layers aren't used: about a centimetre square on a letter page.
const MAX_SOLID_DIFFERENCE: f64 = 0.001;

/// Which box of the editing copy is the page.
#[derive(Clone, Copy)]
enum Page {
    /// An EPS's page is its art's bounding box.
    Art,
    /// A `.ai` is read on its (first and only) artboard.
    Artboard,
}

/// The page (`visible`, the file read as its page) and the place of its area in the layers'
/// document.
struct Frame<'a> {
    page: &'a Document,
    /// The page's area in `page`.
    page_rect: Rect,
    /// The same area in the layers' document.
    rect: Rect,
    /// The page holds all the art that prints (an EPS's page is its art's box, unless it was
    /// saved as its artboard): the layers print nothing outside it.
    whole: bool,
}

/// The document of the editing copy `data`, with the text (and strokes round it) of `visible`, the
/// file read as its page, put in its text objects → the document and its notes. Without a page
/// (a `.ai` saved without its PDF part), its type is made from the file's text document alone and
/// nothing is compared. `outlined`: the page's type was read as outlines, so it has none to put in
/// the text objects.
fn build(data: &[u8], visible: Option<&Document>, page: Page, outlined: bool) -> Result<(Document, Vec<String>), String> {
    let Structure { mut doc, to_doc, bbox, artboard, template, hidden_unread, slot_names, warnings } = ai::read(data)?;
    let (mut shown, mut hidden) = (vec![], vec![]);
    let mut frame = None;
    if let Some(visible) = visible {
        let art_box = match page {
            Page::Art => bbox.map(|[a, b, c, d]| Rect::new(a, b, c, d)).ok_or("its editing data doesn't say where its art is")?,
            Page::Artboard => artboard.ok_or("its editing data doesn't say where its artboard is")?,
        };
        let page_rect = visible.artboards.first().map(|a| a.rect).ok_or("it has no page")?;
        if (art_box.width() - page_rect.width()).abs() > 0.5 || (art_box.height() - page_rect.height()).abs() > 0.5 {
            return Err("its editing data is for a different page".into());
        }
        // The page's area in the layers' document, and the page's type moved there.
        let rect = to_doc.transform_rect_bbox(art_box);
        let shift = Affine::translate(Vec2::new(rect.x0 - page_rect.x0, rect.y0 - page_rect.y0));
        for l in &visible.layers {
            if let Some(c) = l.children() {
                text_objects(c, l.visible, &mut shown, &mut hidden);
            }
        }
        for object in shown.iter_mut().chain(hidden.iter_mut()) {
            for n in object.iter_mut() {
                Arc::make_mut(n).transform(shift, false);
            }
        }
        // An EPS saved as its artboard has the art outside it left out of its page.
        let artboard_page =
            artboard.is_some_and(|a| [a.x0 - art_box.x0, a.y0 - art_box.y0, a.x1 - art_box.x1, a.y1 - art_box.y1].iter().all(|d| d.abs() <= 1.0));
        let whole = matches!(page, Page::Art) && !artboard_page;
        frame = Some(Frame { page: visible, page_rect, rect, whole });
    }
    let mut infos = vec![];
    collect_slots(&doc.layers, true, &mut infos);
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
    let mut empty = 0;
    let mut layers = std::mem::take(&mut doc.layers);
    let mut made = vec![];
    fill_slots(&mut doc, &mut layers, &mut content.into_iter(), &slot_names, &mut empty, &mut made);
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
    if !hidden_unread.is_empty() {
        notes.push(format!("{ART_LEFT_OUT} ({}), left out of them", few(hidden_unread)));
    }
    match frame {
        Some(frame) => {
            let mut view = PageView::new(&frame);
            let unseen = prune_unseen(&frame, &mut view, &mut doc);
            if unseen > 0 {
                notes.push(format!("{unseen} {UNSEEN_TEXT}"));
            }
            notes.extend(compare(&frame, &mut view, &doc)?);
        }
        None => unmark(&mut doc.layers, true, &mut vec![]),
    }
    Ok((doc, notes))
}

/// Marks (in a node's name, until [`prune_unseen`] has looked) the text objects made from the file's
/// text document.
const MADE: &str = "\u{1}made";
/// How many such objects are weighed against the page (each takes a render of the page's size).
const MAX_WEIGHED: usize = 12;
/// The text objects the file keeps that the page doesn't draw.
const UNSEEN_TEXT: &str = "text objects that the file keeps but its page doesn't draw (what is left of type turned to outlines) are left out";
/// How much more of the page (0 to 1) the layers may differ without a text object and still look no worse.
const UNSEEN_MARGIN: f64 = 5e-5;

/// Clear the marks in `nodes` and say which marked objects show, by id.
fn unmark(nodes: &mut [Arc<Node>], shown: bool, found: &mut Vec<(NodeId, Option<Rect>)>) {
    for n in nodes {
        let shown = shown && n.visible;
        if let Some(name) = n.name.as_deref().and_then(|m| m.strip_prefix(MADE)) {
            if shown {
                found.push((n.id, n.visual_bounds()));
            }
            let name = (!name.is_empty()).then(|| name.to_string());
            Arc::make_mut(n).name = name;
        } else if !matches!(n.kind, NodeKind::Compound { .. })
            && let Some(c) = Arc::make_mut(n).children_mut()
        {
            unmark(c, shown, found);
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

/// The text objects made from the file's text document that show on the page's area, where the page
/// has no type to match them, and that the layers look no worse without, are not drawn by the app
/// that wrote the file (which paints every shown text object; it keeps the type of what was turned
/// to outlines): they are taken out. How many.
fn prune_unseen(frame: &Frame<'_>, view: &mut PageView, doc: &mut Document) -> usize {
    let mut found = vec![];
    unmark(&mut doc.layers, true, &mut found);
    let rect = frame.rect;
    if found.is_empty() || found.len() > MAX_WEIGHED {
        return 0;
    }
    let mut with = view.difference(doc, rect).any;
    // Taking a text object out changes only what is drawn in its box. When the art would still
    // differ from the page by more than `compare` allows with every candidate gone, the layers
    // aren't used whatever is pruned: don't draw them again for each one (#758).
    let margin = OUTLINE_REACH + 1.0;
    let boxes: f64 = found.iter().filter_map(|(_, b)| *b).map(|b| b.inflate(margin, margin).intersect(rect).area().max(0.0)).sum();
    if with - boxes / rect.area().max(1e-9) > MAX_DIFFERENCE {
        return 0;
    }
    let mut removed = 0;
    for (id, bounds) in found {
        if bounds.is_none_or(|b| b.intersect(rect).is_zero_area()) {
            continue;
        }
        let mut without = doc.clone();
        if !take_out(&mut without.layers, id, true) {
            continue;
        }
        let d = view.difference(&without, rect).any;
        if d - with <= UNSEEN_MARGIN {
            take_out(&mut doc.layers, id, false);
            with = d;
            removed += 1;
        }
    }
    removed
}

/// How far (points) the art the layers print may reach past a page that holds it all.
const PAST_THE_PAGE: f64 = 1.0;

/// Do the layers' art and the page's own look alike (where the page holds all the art that prints,
/// also round it)? An error says they don't; `Ok` has the note to give when they differ a little.
fn compare(frame: &Frame<'_>, view: &mut PageView, layered: &Document) -> Result<Option<String>, String> {
    let mut diff = view.difference(layered, frame.rect);
    if frame.whole
        && let Some(art) = vectorcraft_render::encode::art_bounds(as_printed(layered).as_ref().unwrap_or(layered))
            .filter(|a| !frame.rect.inflate(PAST_THE_PAGE, PAST_THE_PAGE).contains_rect(*a))
    {
        // The page and the art the layers print beyond it, each in its own document's place.
        let all = frame.rect.union(art);
        let (dx, dy) = (frame.page_rect.x0 - frame.rect.x0, frame.page_rect.y0 - frame.rect.y0);
        let round = difference(frame.page, Rect::new(all.x0 + dx, all.y0 + dy, all.x1 + dx, all.y1 + dy), layered, all);
        diff = Difference { any: diff.any.max(round.any), solid: diff.solid.max(round.solid) };
    }
    if diff.any > MAX_DIFFERENCE || diff.solid > MAX_SOLID_DIFFERENCE {
        return Err(format!("the art on them differs from the page's in {:.1}% of it", diff.any * 100.0));
    }
    Ok((diff.any > NOTABLE_DIFFERENCE).then(|| {
        format!(
            "the art on the layers differs from the file's own page in about {:.1}% of it (brush strokes, live effects and the like that the layers' data doesn't describe)",
            diff.any * 100.0
        )
    }))
}

/// The EPS `visible` (read as its page) read through its editing copy instead: its layers (the
/// hidden ones too), groups, compound paths, clipping groups and artboards, with the text and the
/// strokes around it of the page. When the file has no editing copy, `visible` as it was; when it
/// has one that can't be used, `visible` with a warning that says why.
pub(super) fn layered(ps: &[u8], visible: Imported) -> Imported {
    let Some(data) = ai::eps_data(ps) else { return visible };
    let result = data.and_then(|data| build(&data, Some(&visible.document), Page::Art, false));
    match result {
        Ok((document, mut warnings)) => {
            for w in &visible.warnings {
                if !warnings.contains(w) {
                    warnings.push(w.clone());
                }
            }
            Imported { document, warnings, preview: false }
        }
        Err(why) => {
            let mut v = visible;
            v.warnings.push(format!("{LAYERS_UNREAD} ({why}): it comes in as its page, in one layer"));
            v
        }
    }
}

/// The `.ai` `visible` (read as its PDF part) read through its editing copy (`private`, the joined
/// `AIPrivateData` streams) instead, as [`layered`] reads an EPS: with the layers and objects of the
/// file, its artboards, and the art that lies outside them, which its PDF part doesn't have.
/// `warnings` are the notes of reading the PDF part; the notes returned are those, and the layers'.
/// `outlined`: the PDF part's type was read as outlines (a file whose type shows then comes in as
/// its PDF part).
pub fn layered_ai(private: &[u8], visible: Document, warnings: Vec<String>, outlined: bool) -> (Document, Vec<String>) {
    let Ok(data) = ai::decode_private(private) else { return (visible, warnings) };
    if !data.windows(15).any(|w| w == b"%AI5_BeginLayer") {
        return (visible, warnings);
    }
    match build(&data, Some(&visible), Page::Artboard, outlined) {
        Ok((done, notes)) => {
            let mut all = warnings;
            all.extend(notes.into_iter().filter(|n| !all.contains(n)).collect::<Vec<_>>());
            (done, all)
        }
        Err(why) => {
            let mut all = warnings;
            all.push(format!("{LAYERS_UNREAD} from its editing data ({why}): they come from its PDF part"));
            (visible, all)
        }
    }
}

/// A `.ai` saved without its PDF part (its pages show only a placeholder) from its editing copy
/// alone (`private`, the joined `AIPrivateData` streams) → the document and the import's notes.
pub fn ai_alone(private: &[u8]) -> Result<(Document, Vec<String>), String> {
    let data = ai::decode_private(private)?;
    build(&data, None, Page::Artboard, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{CharStyle, TextObject};

    fn piece(text: &str, x: f64, y: f64) -> Arc<Node> {
        Arc::new(Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(x, y), text, CharStyle::default())))))
    }

    /// Lines that the page draws in pieces, the pieces of one between those of the other, are each
    /// joined, and only their own pieces go.
    #[test]
    fn interleaved_lines_are_joined_without_taking_each_others_pieces() {
        let mut parts = vec![piece("He", 0.0, 0.0), piece("ab", 0.0, -20.0), piece("cd", 16.0, -20.0), piece("llo", 20.0, 0.0)];
        let lines = [("Hello".to_string(), Point::new(0.0, 0.0)), ("abcd".to_string(), Point::new(0.0, -20.0))];
        join_pieces(&mut parts, &lines);
        let texts: Vec<String> = parts.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None }).collect();
        assert_eq!(texts, ["Hello", "abcd"]);
    }
}
