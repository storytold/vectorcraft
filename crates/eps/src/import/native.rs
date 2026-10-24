//! The layers of an Illustrator EPS or `.ai`.
//!
//! Besides the page it prints, such a file carries the app's own copy of the art: an EPS after its
//! `%%EOF`, a `.ai` in its `AIPrivateData` streams (see `ai` for the containers). A file in the
//! legacy format (a `.ai` in PostScript form: versions 3 to 8, and the apps that still write it)
//! is that copy itself: its program has the layers, and the prolog it names defines the operators
//! for printing (other apps name it without including it, so their files can't be run).
//!
//! The copy has what the page doesn't: the layers with their names and options, the groups,
//! compound paths and clipping groups, the objects' names, the hidden objects and layers, the art
//! outside the artboards and every artboard. `ai` reads that structure into a document, with the
//! colours, gradients, transparency and images of the art.
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

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

use vectorcraft_color::BlendMode;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, Scaling, TextKind, Unit};
use vectorcraft_geom::{Affine, PathData, Point, Rect, Vec2};

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
/// Most pairs of a text object and a piece of the page's type compared, and of a glyph the page
/// kept as outlines and a node of the page's type.
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
    /// Its own frame or path, where the file places it ([`ai::text_frames`]).
    art_path: Option<PathData>,
}

/// The text slots of the layers and objects, in order.
fn collect_slots(nodes: &[Arc<Node>], shown: bool, out: &mut Vec<SlotInfo>) {
    for n in nodes {
        let shown = shown && n.visible;
        if let Some(story) = slot_of(n.name.as_deref()) {
            out.push(SlotInfo { story, frame: ai::slot_frame(n.name.as_deref()), shown, art_path: None });
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

/// Does `n` have transparency of its own: an opacity below 100%, a blend mode or isolation?
fn see_through(n: &Node) -> bool {
    n.opacity < 1.0 || n.blend != BlendMode::Normal || n.isolate
}

/// Replace the text slots in `nodes` with what `content` has for each, in order (a slot given none is
/// left out): the page's type takes its place as it is, made type keeps the slot's name. Made type
/// takes the slot's opacity, blend mode, isolation and knockout. The page's type takes them too when
/// the page draws it and the strokes around it with no transparency, the slot's, or the slot's
/// combined with that of the objects around `nodes`; otherwise they keep their own. Where the slot has
/// transparency and the type comes with other art that has none of its own, a group of them takes the
/// slot's instead. `round` has the opacity and blend mode of each object around `nodes`, which the page
/// can have folded into its type. `empty` counts those left out.
fn fill_slots(
    doc: &mut Document,
    nodes: &mut Vec<Arc<Node>>,
    content: &mut std::vec::IntoIter<Content>,
    names: &std::collections::HashMap<NodeId, String>,
    empty: &mut usize,
    made: &mut Vec<(u32, usize, NodeId)>,
    round: &mut Vec<(f32, BlendMode)>,
) {
    let mut i = 0;
    while i < nodes.len() {
        let Some(n) = nodes.get(i) else { break };
        if let Some(story) = slot_of(n.name.as_deref()) {
            let visible = n.visible;
            let name = names.get(&n.id);
            let frame = ai::slot_frame(n.name.as_deref());
            let state = (n.opacity, n.blend, n.isolate, n.knockout);
            let transparent = see_through(n);
            // An art style read onto the slot (fills, strokes and effects round the characters) is the type's.
            let styled = (!n.appearance.items.is_empty() || !n.appearance.effects.is_empty()).then(|| n.appearance.clone());
            let replacement = match content.next() {
                Some(Content::Page(parts)) if !parts.is_empty() => {
                    // The page's type (and the strokes round it) take the slot's place as they are.
                    let mut parts: Vec<Arc<Node>> = parts
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
                    // When the page draws each of them with no transparency, the slot's, or the slot's
                    // combined with that of the objects around it, the slot's takes its place: on the one
                    // part, or on a group of several when the slot has transparency. Otherwise every part
                    // keeps the transparency the page draws it with (a fill's or a stroke's).
                    let above: f32 = round.iter().map(|r| r.0).product();
                    let objects = |p: &Node| {
                        !see_through(p)
                            || (!p.isolate
                                && (p.blend == BlendMode::Normal || p.blend == state.1 || round.iter().any(|r| r.1 == p.blend))
                                && [state.0, state.0 * above].iter().any(|o| (p.opacity - o).abs() < 1e-3))
                    };
                    if parts.iter().all(|p| objects(p)) {
                        for p in &mut parts {
                            let p = Arc::make_mut(p);
                            (p.opacity, p.blend, p.isolate) = (1.0, BlendMode::Normal, false);
                        }
                        if let [p] = parts.as_mut_slice() {
                            let p = Arc::make_mut(p);
                            (p.opacity, p.blend, p.isolate, p.knockout) = state;
                        } else if transparent {
                            let mut group = Node::group(doc.alloc_id(), std::mem::take(&mut parts));
                            (group.opacity, group.blend, group.isolate, group.knockout) = state;
                            parts.push(Arc::new(group));
                        }
                    }
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
                    // Type with an art style is what the page draws as the style's look: it is seen.
                    let seen = seen || styled.is_some();
                    if let Some(appearance) = styled {
                        text.appearance = appearance;
                    }
                    // Type the page shows is kept; other made type is weighed against the page.
                    text.name = if seen { name.cloned() } else { Some(format!("{MADE}{}", name.map_or("", String::as_str))) };
                    let extra: Vec<Arc<Node>> = extra.iter().map(|p| reid(doc, p)).collect();
                    // The type takes the slot's transparency; the page's type kept after it, where it has
                    // none of its own, goes into a group with it that has the transparency instead.
                    let parts: Vec<Arc<Node>> = if transparent && !extra.is_empty() && !extra.iter().any(|p| see_through(p)) {
                        let mut group = Node::group(doc.alloc_id(), std::iter::once(Arc::new(*text)).chain(extra).collect());
                        (group.opacity, group.blend, group.isolate, group.knockout) = state;
                        vec![Arc::new(group)]
                    } else {
                        (text.opacity, text.blend, text.isolate, text.knockout) = state;
                        std::iter::once(Arc::new(*text)).chain(extra).collect()
                    };
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
            let state = (n.opacity, n.blend);
            if n.children().is_some()
                && !matches!(n.kind, NodeKind::Compound { .. })
                && let Some(c) = nodes.get_mut(i).map(Arc::make_mut).and_then(Node::children_mut)
            {
                round.push(state);
                fill_slots(doc, c, content, names, empty, made, round);
                round.pop();
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

/// Do the regions of made type in the slots that `keep` takes cover most of `object`?
fn covered(object: &[Arc<Node>], content: &[Content], keep: impl Fn(usize) -> bool) -> bool {
    let Some(r) = extent(object) else { return false };
    let inside: f64 = content
        .iter()
        .enumerate()
        .filter_map(|(k, c)| match c {
            Content::Made(_, Some(region), ..) => Some(region.intersect(r).area()).filter(|a| *a > 0.0 && keep(k)),
            _ => None,
        })
        .sum();
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
    // The page's objects that are where a slot's lines are, nearest first. A slot that doesn't show
    // has them too: a file's page may draw what its layers have hidden since.
    let mut pairs: Vec<(f64, usize, usize)> = vec![];
    for (k, _) in infos.iter().enumerate() {
        let Some(l) = lines.get(k).and_then(Option::as_ref) else { continue };
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
    // Where the made type that doesn't show lies, by slot.
    let mut hidden_at: Vec<(usize, Rect)> = vec![];
    let mut taller = 0;
    if let Some(t) = template {
        for (k, info) in infos.iter().enumerate() {
            let Some(Some(story)) = stories.get(k) else { continue };
            let Some(mut node) = story.node(NodeId(0), info.frame, t, to_doc) else { continue };
            let mut region = story.region(t, to_doc);
            // Area type and type on a path in the object's own frame: the text document's frame
            // of a copied object keeps its original's path.
            if let (Some(path), NodeKind::Text(text)) = (&info.art_path, &mut node.kind) {
                let size = text.runs.iter().map(|r| r.style.size).fold(0.0, f64::max);
                match &mut text.kind {
                    TextKind::Area { frame } => {
                        *frame = path.clone();
                        region = path.bounds();
                    }
                    TextKind::OnPath { path: on, .. } => {
                        *on = closed_where_ends_meet(path.clone());
                        region = path.bounds().map(|r| r.inflate(size * 1.5, size * 1.5));
                    }
                    TextKind::Point => {}
                }
            }
            if let (false, Some(r)) = (info.shown, region) {
                hidden_at.push((k, r));
            }
            if let NodeKind::Text(text) = &mut node.kind
                && story.fit_first_line(text)
            {
                taller += 1;
            }
            if let Some(c) = content.get_mut(k) {
                *c = Content::Made(Box::new(node), region.filter(|_| info.shown), false, vec![]);
            }
        }
    }
    if taller > 0 {
        warn.push(format!(
            "area type in frames too short for a line of the font it shows in got taller frames ({taller}): their type wouldn't show otherwise"
        ));
    }
    // The file's text document is trusted where it agrees with the page about something (its lines
    // start where the page's type does, or the page has type where it lays type out), or where the
    // page has no type to disagree with.
    let in_region = |object: &[Arc<Node>]| covered(object, &content, |_| true);
    let trusted = owner.iter().any(Option::is_some) || shown.is_empty() || shown.iter().any(|o| in_region(o));
    if !trusted {
        content.iter_mut().for_each(|c| *c = Content::Empty);
    }
    // Is `object` the type of a text object that doesn't show, as the page draws it: in its frame,
    // with its words? (A copied object's lines are where its original's are, so they don't say.)
    let hidden_type = |object: &[Arc<Node>]| {
        let Some(r) = extent(object) else { return false };
        let words: Vec<String> =
            object.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(letters(&t.plain_text())) } else { None }).collect();
        hidden_at.iter().any(|(k, place)| {
            place.intersect(r).area() >= 0.5 * r.area().max(1e-9)
                && lines
                    .get(*k)
                    .and_then(Option::as_ref)
                    .is_some_and(|l| !words.is_empty() && words.iter().all(|w| l.iter().any(|(text, _)| letters(text).contains(w.as_str()))))
        })
    };
    // The letters of each slot's story, in lower case.
    let spelled: Vec<String> = stories.iter().map(|s| s.as_ref().map(|s| letters(s.text()).to_ascii_lowercase()).unwrap_or_default()).collect();
    // For each slot: is its made type where `object` is, with the letters of one of `object`'s texts in
    // its story? All are false unless such slots have the letters of every text between them.
    let spelling = |object: &[Arc<Node>], content: &[Content]| -> Vec<bool> {
        let r = extent(object);
        let words: Vec<String> = object
            .iter()
            .filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(letters(&t.plain_text()).to_ascii_lowercase()) } else { None })
            .filter(|w| !w.is_empty())
            .collect();
        let has = |k: usize| spelled.get(k).is_some_and(|s| words.is_empty() || words.iter().any(|w| s.contains(w.as_str())));
        let near: Vec<bool> = content
            .iter()
            .enumerate()
            .map(|(k, c)| match (c, r) {
                (Content::Made(_, Some(region), ..), Some(r)) => region.intersect(r).area() > 0.0 && has(k),
                _ => false,
            })
            .collect();
        let every = words.iter().all(|w| near.iter().zip(&spelled).any(|(n, s)| *n && s.contains(w.as_str())));
        if every { near } else { vec![] }
    };
    let mut spare: Vec<Vec<Arc<Node>>> = vec![];
    for (j, object) in shown.iter().enumerate() {
        let spells = spelling(object, &content);
        let spells = |k: usize| spells.get(k).copied().unwrap_or(false);
        match owner.get(j).copied().flatten().filter(|_| trusted) {
            // The type of a text object the layers hide: the layers have it, hidden, so the page's
            // isn't another text object's.
            Some(k) if infos.get(k).is_some_and(|i| !i.shown) && matches!(content.get(k), Some(Content::Made(..))) => {}
            Some(k) => match content.get_mut(k) {
                Some(Content::Made(_, _, seen, _)) => *seen = true,
                Some(Content::Page(p)) => p.extend(object.iter().cloned()),
                Some(c) => *c = Content::Page(object.clone()),
                None => {}
            },
            // What lies where made type is and has that type's letters is that type as the page draws it.
            None if covered(object, &content, spells) => {
                let r = extent(object).unwrap_or(Rect::ZERO);
                for (k, c) in content.iter_mut().enumerate() {
                    if let Content::Made(_, Some(region), seen, _) = c
                        && !region.intersect(r).is_zero_area()
                        && spells(k)
                    {
                        *seen = true;
                    }
                }
            }
            None if trusted && hidden_type(object) => {}
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

/// How far apart two pixels' red, green or blue may be for them to be the same colour (a colour
/// and its CMYK equivalent stay within it; a red and a dark red don't).
const SAME_COLOUR: u32 = 64;

/// Are two pixels different colours ([`differ`] also takes a red and a dark red of about the same
/// lightness to be alike)?
fn differ_in_colour(p: &[u8; 4], q: &[u8; 4]) -> bool {
    let (p, q) = (on_white(p), on_white(q));
    p.iter().zip(q).any(|(a, b)| a.abs_diff(b) > SAME_COLOUR)
}

/// How much two documents draw differently: the share (0 to 1) of the pixels that differ, of those
/// that differ away from type, and of those whose neighbors all differ too, away from type (where
/// one draws an object that the other doesn't, not the edges of one drawn a little differently, or
/// type laid out a little differently: a line the page sets in pieces is laid out afresh as one).
#[derive(Clone, Copy)]
struct Difference {
    any: f64,
    art: f64,
    solid: f64,
}

/// The scale the comparisons draw a region at: about 400 pixels along its longer side.
fn compare_scale(r: Rect) -> f64 {
    (400.0 / r.width().max(r.height()).max(1.0)).min(4.0)
}

/// A flattened print page made only of images. Sampling it directly at 400 pixels aliases
/// its fine edges while vector art receives coverage antialiasing. Compare both at a higher
/// resolution and apply the same reconstruction filter; keep the existing difference limits.
fn comparison_samples(doc: &Document) -> u32 {
    let (mut images, mut vector) = (false, false);
    doc.walk(|n| match n.kind {
        NodeKind::Image(_) => images = true,
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Path { clipping: true, .. } => {}
        _ => vector = true,
    });
    if images && !vector { 4 } else { 1 }
}

/// Render for the page check. The comparison scale bounds the long side to 400 px, so the
/// extra samples stay bounded to 1600 px. Filter premultiplied channels, including alpha.
fn comparison_render(
    renderer: &mut vectorcraft_render::Renderer,
    doc: &Document,
    rect: Rect,
    scale: f64,
    samples: u32,
) -> vectorcraft_render::Rendered {
    let rendered = renderer.render_region(doc, rect, scale * f64::from(samples), false);
    if samples == 1 {
        return rendered;
    }
    let (width, height) = match vectorcraft_render::raster_size(rect, scale) {
        Ok(size) => size,
        // A malformed region cannot provide a comparable raster.
        Err(_) => return rendered,
    };
    let Some(src) = image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.pixels) else {
        return vectorcraft_render::Rendered { width: 0, height: 0, pixels: Vec::new() };
    };
    let pixels = image::imageops::resize(&src, width, height, image::imageops::FilterType::Triangle).into_raw();
    vectorcraft_render::Rendered { width, height, pixels }
}

/// The page drawn once, to weigh the layers' art against it: [`Self::difference`] draws the layers
/// with a renderer of their own, which keeps their decoded images between draws (the page's images
/// can share their keys and not their pixels, so the two don't share one).
struct PageView {
    page: vectorcraft_render::Rendered,
    /// Where the page's own type is, in its pixels.
    page_type: Vec<Rect>,
    /// Where objects read with their art style lie, in its pixels: the page has Illustrator's
    /// rendering of the style there, the layers VectorCraft's live effects, and the two aren't
    /// held to look alike.
    restyled: Vec<Rect>,
    layers: vectorcraft_render::Renderer,
    scale: f64,
    samples: u32,
}

impl PageView {
    fn new(frame: &Frame<'_>) -> Self {
        let ra = frame.page_rect;
        let scale = compare_scale(ra);
        // Drawn on nothing and then seen on white: blending modes have no page to blend with, as in
        // the app that wrote the file.
        let samples = comparison_samples(frame.page);
        let page = comparison_render(&mut vectorcraft_render::Renderer::new(), frame.page, ra, scale, samples);
        let mut page_type = vec![];
        type_areas(&frame.page.layers, ra, scale, &mut page_type);
        Self { page, page_type, restyled: vec![], layers: vectorcraft_render::Renderer::new(), scale, samples }
    }

    /// Leave the drawn looks in `looks` (on `region`, the page's area in the layers' document) out
    /// of the comparisons ([`Structure::restyled`](ai::Structure)).
    fn restyled(&mut self, looks: &[Rect], region: Rect) {
        let s = self.scale;
        self.restyled = looks
            .iter()
            .map(|r| r.inflate(OUTLINE_REACH, OUTLINE_REACH))
            .map(|r| Rect::new((r.x0 - region.x0) * s, (r.y0 - region.y0) * s, (r.x1 - region.x0) * s, (r.y1 - region.y0) * s))
            .collect();
    }

    /// How `b` on `rb` (a rectangle the page's size) draws differently from the page ([`compared`]).
    fn difference(&mut self, b: &Document, rb: Rect) -> Difference {
        // The page doesn't print what isn't printed.
        let printed = as_printed(b);
        let b = printed.as_ref().unwrap_or(b);
        let y = comparison_render(&mut self.layers, b, rb, self.scale, self.samples);
        let mut areas = self.page_type.clone();
        type_areas(&b.layers, rb, self.scale, &mut areas);
        compared(&self.page, &y, &areas, &self.restyled)
    }

    /// The share (0 to 1) of the pixels where `b` on `rb` draws another colour than the page
    /// ([`differ_in_colour`]), type included: what taking an object out of the layers changes
    /// there, when it is covered by one of about the same lightness (#1102).
    fn colour_difference(&mut self, b: &Document, rb: Rect) -> f64 {
        let printed = as_printed(b);
        let b = printed.as_ref().unwrap_or(b);
        let y = comparison_render(&mut self.layers, b, rb, self.scale, self.samples);
        if (y.width, y.height) != (self.page.width, self.page.height) {
            return 1.0;
        }
        let differing = self.page.pixels.as_chunks::<4>().0.iter().zip(y.pixels.as_chunks::<4>().0).filter(|(p, q)| differ_in_colour(p, q)).count();
        differing as f64 / (f64::from(y.width) * f64::from(y.height)).max(1.0)
    }
}

/// How `a` on `ra` and `b` on `rb` (rectangles of the same size) draw differently ([`compared`]),
/// for a region other than the page's ([`PageView`] draws that one once).
fn difference(a: &Document, ra: Rect, b: &Document, rb: Rect) -> Difference {
    let printed = as_printed(b);
    let b = printed.as_ref().unwrap_or(b);
    let scale = compare_scale(ra);
    // A renderer each: the two documents' images can share their keys and not their pixels.
    let samples = comparison_samples(a);
    let x = comparison_render(&mut vectorcraft_render::Renderer::new(), a, ra, scale, samples);
    let y = comparison_render(&mut vectorcraft_render::Renderer::new(), b, rb, scale, samples);
    let mut areas = vec![];
    type_areas(&a.layers, ra, scale, &mut areas);
    type_areas(&b.layers, rb, scale, &mut areas);
    compared(&x, &y, &areas, &[])
}

/// How two drawings of the same size differ, each drawn on nothing and seen on white, away from
/// the type in `areas` (pixels), pixels in `skip` counting as alike. Pixels are compared by lightness, and by whether they are inked
/// at all ([`differ`]): the page of an RGB document is written in the colours' CMYK equivalents,
/// and what the comparison looks for is art that is missing or out of place.
fn compared(x: &vectorcraft_render::Rendered, y: &vectorcraft_render::Rendered, areas: &[Rect], skip: &[Rect]) -> Difference {
    if x.width == 0 || x.height == 0 || x.width != y.width || x.height != y.height {
        return Difference { any: 1.0, art: 1.0, solid: 1.0 };
    }
    let (w, h) = (x.width as usize, x.height as usize);
    // Pixels in `skip` count as alike.
    let skipped = in_areas(skip, w, h);
    let differs: Vec<bool> =
        x.pixels.as_chunks::<4>().0.iter().zip(y.pixels.as_chunks::<4>().0).zip(&skipped).map(|((p, q), s)| !*s && differ(p, q)).collect();
    let at = |i: usize, j: usize| j.checked_mul(w).and_then(|row| row.checked_add(i)).and_then(|k| differs.get(k)).copied().unwrap_or(false);
    let typed = in_areas(areas, w, h);
    let art = differs.iter().zip(&typed).filter(|(d, t)| **d && !**t).count();
    // A pixel on the region's edge has neighbours outside it, which don't count as differing.
    let solid = (1..h.saturating_sub(1))
        .flat_map(|j| (1..w.saturating_sub(1)).map(move |i| (i, j)))
        .filter(|&(i, j)| !typed.get(j * w + i).copied().unwrap_or(true) && (j - 1..=j + 1).all(|n| (i - 1..=i + 1).all(|m| at(m, n))))
        .count();
    let area = (w as f64 * h as f64).max(1.0);
    Difference { any: differs.iter().filter(|d| **d).count() as f64 / area, art: art as f64 / area, solid: solid as f64 / area }
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
/// the text objects. `unit`: the size in points of a unit of the page's user space (see
/// [`layered_ai`]).
fn build(data: &[u8], visible: Option<&Document>, page: Page, outlined: bool, trust: bool, unit: f64) -> Result<(Document, Vec<String>), String> {
    // An EPS (its page is its art) draws a Large Canvas page at full size; a `.ai` keeps its art
    // space, as its PDF part may (#2031).
    let Structure { mut doc, to_doc, bbox, artboard, template, hidden_unread, slot_names, restyled, warnings } =
        ai::read(data, matches!(page, Page::Art))?;
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
const MADE: &str = "\u{1}made";
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
fn prune_unseen(frame: &Frame<'_>, view: &mut PageView, doc: &mut Document) -> usize {
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

/// How far (points) the art the layers print may reach past a page that holds it all.
const PAST_THE_PAGE: f64 = 1.0;

/// Do the layers' art and the page's own look alike (where the page holds all the art that prints,
/// also round it)? An error says they don't; `Ok` has the note to give when they differ a little:
/// with the share of the pixels that differ away from type when that is notable, otherwise with the
/// share of those that differ in the type's areas.
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
        diff = Difference { any: diff.any.max(round.any), art: diff.art.max(round.art), solid: diff.solid.max(round.solid) };
    }
    if diff.any > MAX_DIFFERENCE || diff.solid > MAX_SOLID_DIFFERENCE {
        return Err(format!("the art on them differs from the page's in {:.1}% of it", diff.any * 100.0));
    }
    if diff.any <= NOTABLE_DIFFERENCE {
        return Ok(None);
    }
    Ok(Some(if diff.art > NOTABLE_DIFFERENCE {
        format!(
            "the art on the layers differs from the file's own page in about {:.1}% of it (brush strokes, live effects and the like that the layers' data doesn't describe)",
            diff.art * 100.0
        )
    } else {
        format!(
            "the type on the layers differs from the file's own page in about {:.1}% of it (type laid out again from the file's text, or drawn in another font)",
            (diff.any - diff.art) * 100.0
        )
    }))
}

/// The EPS `visible` (read as its page) read through its editing copy instead (its private data, or
/// its program in the legacy format, see [`editing_copy`]): its layers (the
/// hidden ones too), groups, compound paths, clipping groups and artboards, with the text and the
/// strokes around it of the page. When the file has no editing copy, `visible` as it was; when it
/// has one that can't be used, `visible` with a warning that says why.
pub(super) fn layered(ps: &[u8], visible: Imported, trust: bool) -> Imported {
    let Some(data) = editing_copy(ps) else { return visible };
    let result = data.and_then(|data| build(&data, Some(&visible.document), Page::Art, false, trust, 1.0));
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

/// The `.ai` `visible` (read as its PDF part), or a PDF saved with the editing data, read through
/// its editing copy (`private`, the joined `AIPrivateData` or `AIPDFPrivateData` streams) instead,
/// as [`layered`] reads an EPS: with the layers and objects of the file, its artboards (with the
/// PDF part's bleed), and the art that lies outside them, which its PDF part doesn't have.
/// `warnings` are the notes of reading the PDF part; the notes returned are those, then the layers'
/// notes, or a note saying why the layers weren't read when the editing data can't be decompressed
/// or its layers can't be read. The flag returned is `true` when the document is the one read from
/// the layers; else the document is `visible`, its pages moved by `place_pages` when the editing
/// data was decompressed but its layers can't be read. `outlined`: the PDF part's type was read as
/// outlines (a file whose type shows then comes in as its PDF part).
/// `trust`: layers that draw differently from the PDF part are used all the same, with a warning.
/// `unit`: the size in points of a unit of the PDF part's user space, its first page's `UserUnit`,
/// by which its art came in scaled. Editing data whose artboard matches the page only in those
/// units comes in `unit` times as large, and so does the artboard layout `place_pages` takes from
/// it.
pub fn layered_ai(private: &[u8], visible: Document, warnings: Vec<String>, outlined: bool, trust: bool, unit: f64) -> (Document, Vec<String>, bool) {
    let unread = |mut all: Vec<String>, why: &str| {
        all.push(format!("{LAYERS_UNREAD} from its editing data ({why}): they come from its PDF part"));
        all
    };
    let data = match ai::decode_private(private) {
        Ok(data) => data,
        Err(why) => return (visible, unread(warnings, &why), false),
    };
    if !has_layers(&data) {
        return (visible, warnings, false);
    }
    // The PDF part, its pages placed and named as the editing data's artboards are, when the
    // layers can't be used (`why`).
    let from_pdf_part = |mut visible: Document, warnings: Vec<String>, why: &str| {
        place_pages(&ai::artboards(&data), &mut visible, unit);
        let mut all = unread(warnings, why);
        all.extend(ai::variables(&data).note());
        (visible, all, false)
    };
    match build(&data, Some(&visible), Page::Artboard, outlined, trust, unit) {
        // An image too large for the editing data's reader would be missing from the layers, while
        // the PDF part has it: the file comes in as its PDF part instead (#1479).
        Ok((_, notes)) if notes.iter().any(|n| n == ai::IMAGE_TOO_LARGE) => {
            from_pdf_part(visible, warnings, "an image is too large to read from them")
        }
        Ok((mut done, notes)) => {
            done.setup.bleed = visible.setup.bleed;
            let mut all = warnings;
            all.extend(notes.into_iter().filter(|n| !all.contains(n)).collect::<Vec<_>>());
            (done, all, true)
        }
        Err(why) => from_pdf_part(visible, warnings, &why),
    }
}

/// The pages of `doc` (a `.ai` read as its PDF part: a page per artboard, in a row) named as
/// `boards` (the artboards of its editing data, in art space) name them, and moved, with the art on
/// each, to where they are, so the artboards keep their names and layout when the layers can't be
/// read (#1068). Left as they are unless there are as many artboards as pages, each the size of its
/// page in points, or, when `unit` is above 1, each the size of its page in units of `unit` points:
/// then the layout is drawn `unit` times as large.
fn place_pages(boards: &[(String, Rect)], doc: &mut Document, unit: f64) {
    if boards.is_empty() || boards.len() != doc.artboards.len() {
        return;
    }
    let differ = |k: f64| {
        boards
            .iter()
            .zip(&doc.artboards)
            .any(|((_, b), a)| (b.width() - a.rect.width() / k).abs() > 0.5 || (b.height() - a.rect.height() / k).abs() > 0.5)
    };
    let k = if !differ(1.0) {
        1.0
    } else if unit.is_finite() && unit > 1.0 && !differ(unit) {
        unit
    } else {
        return;
    };
    for ((name, _), a) in boards.iter().zip(&mut doc.artboards) {
        a.name.clone_from(name);
    }
    let [(_, first), _, ..] = boards else { return };
    // Art space (y up) → the document, the first artboard's top left at the origin, as `read` has it,
    // `k` points a unit.
    let to_doc = Affine::new([k, 0.0, 0.0, -k, -first.x0 * k, first.y1 * k]);
    let pages: Vec<Rect> = doc.artboards.iter().map(|a| a.rect).collect();
    let moves: Vec<Vec2> = boards.iter().zip(&pages).map(|((_, b), page)| to_doc.transform_rect_bbox(*b).origin() - page.origin()).collect();
    for layer in &mut doc.layers {
        place_art(Arc::make_mut(layer), &pages, &moves);
    }
    for (a, v) in doc.artboards.iter_mut().zip(&moves) {
        a.rect = a.rect + *v;
    }
}

/// The objects in `layer` (and its sublayers) moved by the move of the page they are on.
fn place_art(layer: &mut Node, pages: &[Rect], moves: &[Vec2]) {
    let Some(children) = layer.children_mut() else { return };
    for child in children {
        if matches!(child.kind, NodeKind::Layer { .. }) {
            place_art(Arc::make_mut(child), pages, moves);
            continue;
        }
        let Some(r) = child.visual_bounds() else { continue };
        // The page it is on: the one with most of it (pages keep only the art that reaches into
        // them), else the nearest.
        let fit = |p: &Rect| (p.intersect(r).area().max(0.0), -(p.center() - r.center()).hypot());
        let Some((i, _)) = pages.iter().map(fit).enumerate().max_by(|(_, a), (_, b)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))) else { continue };
        if let Some(v) = moves.get(i).filter(|v| **v != Vec2::ZERO) {
            Arc::make_mut(child).transform(Affine::translate(*v), false);
        }
    }
}

/// A `.ai` saved without its PDF part (its pages show only a placeholder) from its editing copy
/// alone (`private`, the joined `AIPrivateData` streams) → the document and the import's notes.
pub fn ai_alone(private: &[u8]) -> Result<(Document, Vec<String>), String> {
    let data = ai::decode_private(private)?;
    build(&data, None, Page::Artboard, false, false, 1.0)
}

/// Does the editing data (or a program in the legacy format) have layers?
fn has_layers(data: &[u8]) -> bool {
    data.windows(15).any(|w| w == b"%AI5_BeginLayer")
}

/// The editing copy of the art in the PostScript `ps`: an EPS's private data, else the program
/// itself when it has layers (the legacy format). `None` without one.
fn editing_copy(ps: &[u8]) -> Option<Result<Cow<'_, [u8]>, String>> {
    match ai::eps_data(ps) {
        Some(data) => Some(data.map(Cow::Owned)),
        None => has_layers(ps).then_some(Ok(Cow::Borrowed(ps))),
    }
}

/// A file in the legacy format whose page can't be read (a `.ai` in PostScript form that names its
/// prolog without including it) from its layers alone (`ps`: its program, which has them) → the
/// document and the import's notes. A file whose header says it is in the format (`illustrator`)
/// but whose program has no layers (Illustrator 3 had none; image editors still export their
/// paths so, storytold/photocraft#6036) is read with its art on one layer. `None` when the program
/// has no layers and isn't in the format: other PostScript isn't read as the format's operators.
pub(super) fn program_alone(ps: &[u8], illustrator: bool) -> Option<Result<(Document, Vec<String>), String>> {
    if ai::has_eps_data(ps) {
        return None;
    }
    if has_layers(ps) {
        return Some(build(ps, None, Page::Art, false, false, 1.0));
    }
    let program = one_layer(ps).filter(|_| illustrator)?;
    Some(build(&program, None, Page::Art, false, false, 1.0))
}

/// `ps` with its art (from its `%%EndSetup` to its `%%PageTrailer`, else its `%%Trailer`, else its
/// end) on a layer that shows, prints and isn't locked, as the version 5 format writes one. `None`
/// without a `%%EndSetup`: where the art starts isn't known.
fn one_layer(ps: &[u8]) -> Option<Vec<u8>> {
    let setup = line_with(ps, 0, b"%%EndSetup")?;
    let art = line_with(ps, setup + 1, b"").unwrap_or(ps.len());
    let end = line_with(ps, art, b"%%PageTrailer").or_else(|| line_with(ps, art, b"%%Trailer")).unwrap_or(ps.len());
    let mut out = Vec::with_capacity(ps.len() + 80);
    out.extend_from_slice(ps.get(..art)?);
    out.extend_from_slice(b"\n%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n(Layer 1) Ln\n");
    out.extend_from_slice(ps.get(art..end)?);
    out.extend_from_slice(b"\nLB\n%AI5_EndLayer--\n");
    out.extend_from_slice(ps.get(end..)?);
    Some(out)
}

/// Where the first line of `ps` from `from` on that starts with `key` starts.
fn line_with(ps: &[u8], from: usize, key: &[u8]) -> Option<usize> {
    (from..ps.len()).find(|&i| (i == 0 || matches!(ps.get(i - 1), Some(b'\n' | b'\r'))) && ps.get(i..).is_some_and(|rest| rest.starts_with(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_doc::{CharStyle, TextObject};

    /// `doc` with a square ring round (20, 20)–(80, 80) (or at `at`), stroked `color` `width` wide,
    /// named `name`.
    fn ring(doc: &mut Document, name: Option<&str>, color: vectorcraft_color::Color, width: f64, at: Rect) {
        let stroke = vectorcraft_color::Paint::solid(color);
        let mut n = Node::path(
            doc.alloc_id(),
            vectorcraft_geom::shapes::rectangle(at),
            vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, stroke, width),
        );
        n.name = name.map(str::to_string);
        let layer = doc.layers[0].id;
        doc.insert(Some(layer), usize::MAX, n).unwrap();
    }

    /// A text object's red stroke drawn over a dark red one of about the same lightness is drawn
    /// by the page: it isn't taken out as unseen (#1102). An object the page doesn't draw still is.
    #[test]
    fn a_stroke_over_one_as_light_is_not_taken_for_unseen() {
        let (dark, red) = (vectorcraft_color::Color::rgb(0.55, 0.0, 0.0), vectorcraft_color::Color::rgb(1.0, 0.0, 0.0));
        let square = Rect::new(20.0, 20.0, 80.0, 80.0);
        let mut page = Document::new(100.0, 100.0);
        ring(&mut page, None, dark, 12.0, square);
        ring(&mut page, None, red, 6.0, square);
        let mut layers = Document::new(100.0, 100.0);
        ring(&mut layers, Some(MADE), dark, 12.0, square);
        ring(&mut layers, Some(MADE), red, 6.0, square);
        ring(&mut layers, Some(MADE), red, 2.0, Rect::new(40.0, 40.0, 60.0, 60.0));
        let rect = page.artboards[0].rect;
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false };
        let mut view = PageView::new(&frame);
        assert_eq!(prune_unseen(&frame, &mut view, &mut layers), 1, "only the ring the page doesn't draw");
        let widths: Vec<f64> = layers.layers[0].children().unwrap().iter().filter_map(|n| n.appearance.stroke().map(|s| s.width)).collect();
        assert_eq!(widths, [12.0, 6.0]);
        // Taken by lightness alone, the red and the dark red are alike.
        let px = |c: vectorcraft_color::Color| {
            let [r, g, b] = c.to_rgb().map(|v| (v * 255.0).round() as u8);
            [r, g, b, 255]
        };
        assert!(!differ(&px(dark), &px(red)) && differ_in_colour(&px(dark), &px(red)));
    }

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
    /// A flattened page samples a fine stripe pattern once per comparison pixel; that
    /// aliases to white although vector coverage is gray. Shared minification avoids the
    /// false fallback, while genuinely missing art must still be rejected.
    #[test]
    fn raster_page_comparison_filters_minification_without_accepting_missing_art() {
        use vectorcraft_color::{Color, Paint};
        use vectorcraft_doc::{Appearance, ImageBlob, ImageObject};
        let mut vector = Document::new(1000.0, 500.0);
        let layer = vector.layers[0].id;
        for i in 0..400 {
            let x = f64::from(i) * 2.5 + 0.5;
            let n = Node::path(
                vector.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 1.0, 500.0)),
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            );
            vector.insert(Some(layer), usize::MAX, n).unwrap();
        }
        let rect = vector.artboards[0].rect;
        let source = vectorcraft_render::Renderer::new().render_region(&vector, rect, 1.6, true);
        let mut png = Vec::new();
        image::RgbaImage::from_raw(source.width, source.height, source.to_straight())
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        let mut page = Document::new(1000.0, 500.0);
        page.images.insert(key.clone(), blob);
        let image = Node::new(
            page.alloc_id(),
            NodeKind::Image(Box::new(ImageObject {
                key,
                width: source.width,
                height: source.height,
                xf: Affine::scale(1.0 / 1.6),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        );
        page.insert(Some(page.layers[0].id), usize::MAX, image).unwrap();
        assert_eq!(comparison_samples(&page), 4);
        assert_eq!(comparison_samples(&vector), 1, "ordinary vector pages retain their comparison");
        let mut renderer = vectorcraft_render::Renderer::new();
        let a = comparison_render(&mut renderer, &page, rect, 0.4, 1);
        let b = comparison_render(&mut vectorcraft_render::Renderer::new(), &vector, rect, 0.4, 1);
        assert!(compared(&a, &b, &[], &[]).solid > MAX_SOLID_DIFFERENCE, "the old point sampling falsely refused this page");
        let frame = Frame { page: &page, page_rect: rect, rect, whole: false };
        let mut view = PageView::new(&frame);
        assert!(compare(&frame, &mut view, &vector).is_ok());
        assert!(compare(&frame, &mut view, &Document::new(1000.0, 500.0)).is_err(), "missing artwork remains an error");
        let shifted = Rect::new(0.0, 0.0, 500.0, 500.0);
        assert!(view.difference(&vector, shifted).any > MAX_DIFFERENCE, "wrong framing remains an error");
    }
}

/// `path` with each open subpath whose last anchor lies on its first (within a thousandth of a
/// point) closed there: the object's own path of type round a closed path is written open, ending
/// on its start, and type on an open path stops at its end, so it had no room to run past it.
fn closed_where_ends_meet(mut path: PathData) -> PathData {
    for sp in &mut path.subpaths {
        let (Some(first), Some(last)) = (sp.anchors.first(), sp.anchors.last()) else { continue };
        if sp.closed || sp.anchors.len() < 3 || first.p.distance(last.p) >= 1e-3 {
            continue;
        }
        if let Some(l) = sp.anchors.pop()
            && let Some(a0) = sp.anchors.first_mut()
        {
            a0.h_in = l.h_in;
        }
        sp.closed = true;
    }
    path
}

#[cfg(test)]
mod closed_ends_tests {
    use super::*;

    /// A path drawn round to its start (the last anchor on the first) is closed there, its first
    /// anchor taking the last one's incoming handle; one whose ends are apart stays open.
    #[test]
    fn a_path_ending_on_its_start_is_closed() {
        let mut bp = kurbo::BezPath::new();
        bp.move_to((100.0, 0.0));
        bp.curve_to((100.0, 55.0), (55.0, 100.0), (0.0, 100.0));
        bp.curve_to((-55.0, 100.0), (-100.0, 55.0), (-100.0, 0.0));
        bp.curve_to((-100.0, -55.0), (-55.0, -100.0), (0.0, -100.0));
        bp.curve_to((55.0, -100.0), (100.0, -55.0), (100.0, 0.0001));
        let closed = closed_where_ends_meet(PathData::from_bezpath(&bp));
        let sp = &closed.subpaths[0];
        assert!(sp.closed && sp.anchors.len() == 4, "{sp:?}");
        assert_eq!(sp.anchors[0].h_in, Point::new(100.0, -55.0));
        let mut open = kurbo::BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((50.0, 0.0));
        open.line_to((50.0, 50.0));
        assert!(!closed_where_ends_meet(PathData::from_bezpath(&open)).subpaths[0].closed);
    }
}
