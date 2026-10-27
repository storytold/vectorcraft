use super::build::MADE;
use super::*;

/// The notes that say the import left out something the file has (see [`is_loss`]).
pub(super) const TEXT_LEFT_OUT: &str = "text objects on hidden layers or hidden objects couldn't be read, so they are left out";

/// The same for a file without a page to take shown type from.
pub(super) const TYPE_LEFT_OUT: &str = "text objects of kinds VectorCraft doesn't read from the file's text document yet are left out";

pub(super) const ART_LEFT_OUT: &str = "hidden layers have art this can't read";

pub(super) const LAYERS_UNREAD: &str = "the file's layers weren't read";

/// Does this import note say that the file has something the document doesn't (hidden text, art or
/// layers that could not be read)? Writing the document over the file would lose it for good.
pub fn is_loss(note: &str) -> bool {
    note.ends_with(TEXT_LEFT_OUT)
        || note.ends_with(TYPE_LEFT_OUT)
        || [ART_LEFT_OUT, LAYERS_UNREAD, crate::import::graphics::TOO_MUCH, crate::import::graphics::FAR_AWAY].iter().any(|n| note.starts_with(n))
}

/// Most text objects in the layers, and most pieces of type in one place on the page, that are matched
/// to each other (more than this and the layers aren't used).
pub(super) const MAX_SLOTS: usize = 5000;

const MAX_PIECES: usize = 2000;

/// Most bytes of text in the text objects of the layers (a story named twice counts twice).
pub(super) const MAX_TEXT: usize = 16 << 20;

/// Most pairs of a text object and a piece of the page's type compared, and of a glyph the page
/// kept as outlines and a node of the page's type.
pub(super) const MAX_PAIRS: usize = 4_000_000;

/// `names` for a note: the first few, and how many more.
pub(super) fn few(names: BTreeSet<String>) -> String {
    const SHOWN: usize = 5;
    let more = names.len().saturating_sub(SHOWN);
    let mut list: Vec<String> = names.into_iter().take(SHOWN).collect();
    if more > 0 {
        list.push(format!("{more} more"));
    }
    list.join(", ")
}

/// A text slot of the layers.
pub(super) struct SlotInfo {
    pub(super) story: Option<u32>,
    /// Which of the story's frames it is.
    pub(super) frame: usize,
    /// It, its layer and the objects round it show.
    pub(super) shown: bool,
    /// Its own frame or path, where the file places it ([`ai::text_frames`]).
    pub(super) art_path: Option<PathData>,
}

/// The text slots of the layers and objects, in order.
pub(super) fn collect_slots(nodes: &[Arc<Node>], shown: bool, out: &mut Vec<SlotInfo>) {
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
pub(super) const OUTLINE_REACH: f64 = 6.0;

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
pub(super) fn text_objects(nodes: &[Arc<Node>], on: bool, shown: &mut Vec<Vec<Arc<Node>>>, hidden: &mut Vec<Vec<Arc<Node>>>) {
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
pub(super) fn assign(objects: &[Vec<Arc<Node>>], slots: usize) -> Vec<Vec<&[Arc<Node>]>> {
    let mut given: Vec<Vec<&[Arc<Node>]>> = vec![vec![]; slots];
    for (j, o) in objects.iter().enumerate() {
        if let Some(at) = given.get_mut(((2 * j + 1) * slots / (2 * objects.len())).min(slots.saturating_sub(1))) {
            at.push(o);
        }
    }
    given
}

/// What goes into a text slot.
pub(super) enum Content {
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
pub(super) fn fill_slots(
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
pub(super) fn letters(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphanumeric).collect()
}

/// Does `n` hold type that is where `story` lays one of its lines out: the line, or a piece of it
/// (the page splits a line where its kerning or its styles change), on that line's baseline and
/// after its start? How far from where the line starts it is, if so.
pub(super) fn same_type(n: &Node, lines: &[(String, Point)]) -> Option<f64> {
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
pub(super) fn join_pieces(parts: &mut Vec<Arc<Node>>, lines: &[(String, Point)]) {
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
pub(super) fn covered(object: &[Arc<Node>], content: &[Content], keep: impl Fn(usize) -> bool) -> bool {
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
pub(super) fn extent(parts: &[Arc<Node>]) -> Option<Rect> {
    parts.iter().filter_map(|n| n.visual_bounds()).reduce(|a, b| a.union(b))
}
