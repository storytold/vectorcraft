use super::layers::closed_where_ends_meet;
use super::slots::{
    Content, MAX_PAIRS, OUTLINE_REACH, PAGE_TYPE_LEFT_OUT, PAGE_TYPE_UNCHECKED, SlotInfo, assign, covered, drawn_box, extent, join_pieces, letters,
    same_type, word_pieces, words,
};
use super::{Affine, Arc, Node, NodeId, NodeKind, Point, Rect, Story, TextKind};

/// A slot's type made from the file's text document, and roughly where it lies on the document.
pub(super) type Made = (Box<Node>, Option<Rect>);

/// Match page objects to the nearest slot that `open` (the slots whose type wasn't made) has and
/// whose story describes them, including hidden slots.
fn match_page_objects(infos: &[SlotInfo], lines: &[Option<Vec<(String, Point)>>], open: &[bool], shown: &[Vec<Arc<Node>>]) -> Vec<Option<usize>> {
    let mut pairs: Vec<(f64, usize, usize)> = vec![];
    for (k, _) in infos.iter().enumerate() {
        if !open.get(k).copied().unwrap_or(false) {
            continue;
        }
        let Some(l) = lines.get(k).and_then(Option::as_ref) else { continue };
        for (j, object) in shown.iter().enumerate() {
            if let Some(d) = object.iter().filter_map(|n| same_type(n, l)).min_by(f64::total_cmp) {
                pairs.push((d, k, j));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut owner = vec![None; shown.len()];
    for (_, k, j) in pairs {
        if let Some(o) = owner.get_mut(j).filter(|o| o.is_none()) {
            *o = Some(k);
        }
    }
    owner
}

fn frame_text_from_art(info: &SlotInfo, node: &mut Node, region: &mut Option<Rect>) {
    if let (Some(path), NodeKind::Text(text)) = (&info.art_path, &mut node.kind) {
        let size = text.runs.iter().map(|r| r.style.size).fold(0.0, f64::max);
        match &mut text.kind {
            TextKind::Area { frame } => {
                *frame = path.clone();
                *region = path.bounds();
            }
            TextKind::OnPath { path: on, .. } => {
                *on = closed_where_ends_meet(path.clone());
                *region = path.bounds().map(|r| r.inflate(size * 1.5, size * 1.5));
            }
            TextKind::Point => {}
        }
    }
}

/// Each of `infos`' slots' type made from the file's text document, once: `None` where the text
/// document can't make it (no story, a frame of a kind it doesn't read, mirrored type, no text).
pub(super) fn make_slots(
    infos: &[SlotInfo],
    stories: &[Option<Arc<Story>>],
    template: Option<(f64, f64)>,
    to_doc: Affine,
    warn: &mut Vec<String>,
) -> Vec<Option<Made>> {
    let (mut taller, mut deeper, mut tabs_left_out, mut kinsoku_left_out) = (0, 0, false, false);
    let made = infos
        .iter()
        .enumerate()
        .map(|(k, info)| {
            let t = template?;
            let story = stories.get(k)?.as_ref()?;
            let node = story.node(NodeId(0), info.frame, t, to_doc)?;
            tabs_left_out |= story.tabs_left_out();
            kinsoku_left_out |= story.kinsoku_sets_left_out();
            let (mut node, region) = placed(info, story, node, t, to_doc);
            if let NodeKind::Text(text) = &mut node.kind
                && story.fit_first_line(text)
            {
                taller += 1;
            }
            if let NodeKind::Text(text) = &mut node.kind
                && story.fit_last_lines(text)
            {
                deeper += 1;
            }
            Some((Box::new(node), region))
        })
        .collect();
    if taller > 0 {
        warn.push(format!(
            "area type in frames too short for a line of the font it shows in got taller frames ({taller}): their type wouldn't show otherwise"
        ));
    }
    if deeper > 0 {
        warn.push(format!(
            "area type whose last lines the file sets in frames too short for them here got taller frames ({deeper}): those lines wouldn't show otherwise"
        ));
    }
    if tabs_left_out {
        warn.push(crate::import::ate::TABS_LEFT_OUT.into());
    }
    if kinsoku_left_out {
        warn.push(crate::import::ate::KINSOKU_SETS_LEFT_OUT.into());
    }
    made
}

/// The empty text object a slot of a blank story in an area frame or on a path comes in as, placed
/// as [`make_slots`] places made type: its frame or path, the object's own where it has one
/// ([`Story::blank_frame_node`]). `None` for any other slot.
pub(super) fn blank_frame(info: &SlotInfo, story: &Story, template: (f64, f64), to_doc: Affine) -> Option<Made> {
    let node = story.blank_frame_node(NodeId(0), info.frame, template, to_doc)?;
    let (node, region) = placed(info, story, node, template, to_doc);
    Some((Box::new(node), region))
}

/// `node`, made from `story` for slot `info`, placed where the file has it, with the region it
/// draws in: the first line where the file has it, in the text document's frame (that of a copied
/// object is its original's, as its lines are); area type and type on a path in the object's own
/// frame (the text document's frame of a copied object keeps its original's path); type on a path
/// flipped as its frame keeps it.
fn placed(info: &SlotInfo, story: &Story, mut node: Node, template: (f64, f64), to_doc: Affine) -> (Node, Option<Rect>) {
    let mut region = story.region(template, to_doc);
    story.place_first_line(&mut node, info.frame, template, to_doc);
    frame_text_from_art(info, &mut node, &mut region);
    story.flip_on_path(&mut node, info.frame);
    (node, region)
}

/// Does the file's text document make the type of every slot that shows (at least one does)? Type
/// that a look draws doesn't count: only the page has it.
pub(super) fn readable_all(infos: &[SlotInfo], made: &[Option<Made>]) -> bool {
    let counted = |i: &SlotInfo| i.shown && i.look.is_none();
    infos.iter().any(counted) && infos.iter().zip(made).all(|(i, m)| !counted(i) || m.is_some())
}

/// Each slot's made type, where its slot is (the page's type that shows isn't put anywhere, but for
/// the type of the copies that looks draw, see [`fill_look_slots`]). Shown made type with none of
/// the page's type where it draws is weighed against the page (see `prune_unseen`); the page's type
/// that no made type takes, shown or not, is left out with a note in `warn`. Slots that don't show
/// and whose type isn't made take the page's type that doesn't show (`pages`: the page's text
/// objects that show and that don't), by painting order.
pub(super) fn made_only(
    infos: &[SlotInfo],
    stories: &[Option<Arc<Story>>],
    made: Vec<Option<Made>>,
    pages: [&[Vec<Arc<Node>>]; 2],
    warn: &mut Vec<String>,
) -> Vec<Content> {
    let [shown, hidden] = pages;
    // Every text object that shows has made type: each is matched with the page's type where it draws.
    let mut content = made_content(infos, made, true);
    // The page's type where no made type draws, in the copies that looks draw, is theirs (past
    // `MAX_PAIRS` comparisons it isn't looked for).
    let looks = infos.iter().any(|i| i.shown && i.look.is_some());
    let mut taken = std::collections::HashSet::new();
    if looks && infos.len().saturating_mul(shown.len()) <= MAX_PAIRS {
        let spelled: Vec<String> =
            content.iter().map(|c| if let Content::Made(node, ..) = c { spelling(&plain_text(node)) } else { String::new() }).collect();
        let made_type = |o: &[Arc<Node>]| covered(o, &content, |_| true) && has_made_words(o, &content, &spelled);
        let mut spare: Vec<Vec<Arc<Node>>> = shown.iter().filter(|o| !made_type(o)).cloned().collect();
        fill_look_slots(infos, &mut content, &mut spare);
        for c in &content {
            if let Content::Page(parts) = c {
                taken.extend(parts.iter().map(|n| n.id));
            }
        }
    }
    let rest: Vec<Vec<Arc<Node>>>;
    let untaken = if taken.is_empty() {
        shown
    } else {
        rest = shown.iter().filter(|o| !o.iter().any(|n| taken.contains(&n.id))).cloned().collect();
        &rest
    };
    match see_page_type(&mut content, untaken, true) {
        Some(0) => {}
        Some(_) => warn.push(PAGE_TYPE_LEFT_OUT.into()),
        None => warn.push(PAGE_TYPE_UNCHECKED.into()),
    }
    drop_look_slots(infos, &mut content);
    fill_hidden(infos, stories, &mut content, hidden);
    content
}

/// Give the page's type in `spare` that lies in a copy that a layer's or group's look draws (at least
/// half of it in the box of the copy's other art, or round it as far as strokes round type reach; of
/// overlapping copies, the one that holds the most of it, else the smallest) to that copy's text
/// slots that show, by painting order, taking it out of `spare`. Type further out than that stays in
/// `spare`. The other text objects in a copy's group that have
/// nothing yet take its type with them, in their order. Where the page's type outside the copies is
/// too little for the other text objects that have nothing yet, a copy with more of the page's text
/// objects than slots leaves the extra ones in `spare` for them. The look's other slots are dropped
/// ([`drop_look_slots`]).
fn fill_look_slots(infos: &[SlotInfo], content: &mut [Content], spare: &mut Vec<Vec<Arc<Node>>>) {
    // Each copy's group and box, the slots in it that show and have nothing yet, in order, and the
    // page's text objects (by their place in `spare`) it holds.
    // A group has one box: that of its art, or of the group around it.
    type Copy = (Option<NodeId>, Rect, Vec<usize>, Vec<usize>);
    let mut copies: Vec<Copy> = vec![];
    let mut by_group: std::collections::HashMap<Option<NodeId>, usize> = Default::default();
    for info in infos {
        let Some(copy) = info.look.filter(|r| info.shown && !r.is_zero_area()) else { continue };
        by_group.entry(info.group).or_insert_with(|| {
            copies.push((info.group, copy, vec![], vec![]));
            copies.len() - 1
        });
    }
    if copies.is_empty() || spare.is_empty() {
        return drop_look_slots(infos, content);
    }
    for (k, (info, c)) in infos.iter().zip(content.iter()).enumerate() {
        if let Some(copy) = by_group.get(&info.group).and_then(|i| copies.get_mut(*i)).filter(|_| info.shown && matches!(c, Content::Empty)) {
            copy.2.push(k);
        }
    }
    for (j, object) in spare.iter().enumerate() {
        let Some(r) = extent(object) else { continue };
        let share = |copy: &Rect| copy.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(r).area() / r.area().max(1e-9);
        let best = copies
            .iter_mut()
            .map(|c| (share(&c.1), c))
            .filter(|(s, c)| *s >= 0.5 && !c.2.is_empty())
            .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.1.area().total_cmp(&a.1.1.area())));
        if let Some((_, (.., held))) = best {
            held.push(j);
        }
    }
    // Copies with the same box share its type, by painting order.
    let mut boxes: Vec<(Rect, Vec<usize>, Vec<usize>)> = vec![];
    for (_, copy, slots, held) in copies {
        match boxes.iter_mut().find(|b| b.0 == copy) {
            Some(b) => {
                b.1.extend(slots);
                b.2.extend(held);
            }
            None => boxes.push((copy, slots, held)),
        }
    }
    // Other text objects that show and have nothing yet, outside the copies.
    let in_copy: std::collections::BTreeSet<usize> = boxes.iter().flat_map(|b| b.1.iter().copied()).collect();
    let others: Vec<usize> = infos
        .iter()
        .zip(content.iter())
        .enumerate()
        .filter(|(k, (i, c))| i.shown && i.look.is_none() && matches!(c, Content::Empty) && !in_copy.contains(k))
        .map(|(k, _)| k)
        .collect();
    // How many of them the page's type outside the copies leaves without any.
    let outside = spare.len().saturating_sub(boxes.iter().map(|b| b.2.len()).sum());
    let mut short = others.len().saturating_sub(outside);
    let mut taken = vec![false; spare.len()];
    for (_, slots, held) in &mut boxes {
        held.sort_unstable();
        // The page paints in the order of the layers: the extra ones go first to the other text objects
        // before the copy, from the start, then to those after it, from the end.
        let first = slots.first().copied().unwrap_or(0);
        let extra = held.len().saturating_sub(slots.len()).min(short);
        short -= extra;
        let before = others.iter().filter(|k| **k < first).count().min(extra);
        held.drain(..before);
        held.truncate(held.len() - (extra - before));
        let objects: Vec<Vec<Arc<Node>>> = held.iter().filter_map(|j| spare.get(*j).cloned()).collect();
        for j in held.iter() {
            if let Some(t) = taken.get_mut(*j) {
                *t = true;
            }
        }
        for (slot, parts) in slots.iter().zip(assign(&objects, slots.len())) {
            if let Some(c) = content.get_mut(*slot).filter(|_| !parts.is_empty()) {
                *c = Content::Page(parts.into_iter().flat_map(|p| p.iter().cloned()).collect());
            }
        }
    }
    let mut kept = taken.iter();
    spare.retain(|_| !kept.next().copied().unwrap_or(false));
    drop_look_slots(infos, content);
}

/// Has made type that shows and draws where the page's text object `object` lies every word of it
/// ([`spelling`], each slot's in `spelled`), in any order?
fn has_made_words(object: &[Arc<Node>], content: &[Content], spelled: &[String]) -> bool {
    let Some(r) = extent(object) else { return true };
    let near: Vec<&String> = content
        .iter()
        .zip(spelled)
        .filter(|(c, _)| matches!(c, Content::Made(_, Some(region), _) if !region.intersect(r).is_zero_area()))
        .map(|(_, s)| s)
        .collect();
    object_words(object).iter().all(|w| near.iter().any(|s| s.contains(w.as_str())))
}

/// The text slots for type that a look draws and that have nothing have no type of their own to hold
/// (the page has it, or the text document of such type isn't read): they go, and aren't counted as
/// text objects that couldn't be read.
fn drop_look_slots(infos: &[SlotInfo], content: &mut [Content]) {
    for (info, c) in infos.iter().zip(content) {
        if info.look.is_some() && matches!(c, Content::Empty) {
            *c = Content::Merged;
        }
    }
}

/// The letters and digits of `s`, in any script, in lower case: what is compared of the page's type
/// and made type (the page may spell a mark another way).
fn spelling(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The text of `n`, if it is a text object.
fn plain_text(n: &Node) -> String {
    if let NodeKind::Text(t) = &n.kind { t.plain_text() } else { String::new() }
}

/// The words of the page's text object `o` (see [`word_pieces`]), each as its [`spelling`], the
/// empty ones left out.
fn object_words(o: &[Arc<Node>]) -> Vec<String> {
    o.iter().flat_map(|n| word_pieces(&plain_text(n)).map(spelling).collect::<Vec<_>>()).filter(|w| !w.is_empty()).collect()
}

/// Each slot's made type as its content, else nothing yet. Made type that shows has where it draws,
/// or where its story lies when it draws nothing. `laid`: point type is laid out for its box
/// ([`drawn_box`]); without it the box is that of type without a layout, as it always was.
fn made_content(infos: &[SlotInfo], made: Vec<Option<Made>>, laid: bool) -> Vec<Content> {
    infos
        .iter()
        .zip(made)
        .map(|(info, m)| match m {
            Some((node, region)) => {
                // Type that doesn't show has no region: it isn't laid out for one.
                let own = || if laid { drawn_box(&node) } else { node.visual_bounds() };
                let drawn = info.shown.then(|| own().filter(|b| !b.is_zero_area() && b.is_finite()).or(region)).flatten();
                Content::Made(node, drawn, false)
            }
            None => Content::Empty,
        })
        .collect()
}

/// Mark the made type that has at least half of a piece of the page's type where it draws (or round
/// it, as far as strokes round type reach) as type the page draws. How many of the page's text
/// objects no made type takes, shown or not: none that holds a piece of the object has its words
/// ([`spelling`]). Past `MAX_PAIRS` comparisons nothing is marked or counted (`None`).
///
/// The page can draw the type of several text objects as one piece (one line, the text objects
/// apart along it): none of them holds half of the piece. With `lines` (not from [`plan`], which
/// puts the page's pieces into slots too), after the pass above, shown made type
/// that holds no piece takes the words of an object that are still left where the page draws them
/// ([`take_words_drawn_at`]): the words that follow one another along the piece, written as the
/// made type writes them, in about the place where it draws. Where such text objects take every
/// word the object has left, they are marked and the words are no longer left; where some word
/// stays, nothing changes. Words another text object took, words drawn elsewhere along the piece,
/// and words written another way (other case, other marks) aren't taken: type the page doesn't
/// draw there stays unmarked, to be weighed against the page (see `prune_unseen`).
fn see_page_type(content: &mut [Content], shown: &[Vec<Arc<Node>>], lines: bool) -> Option<usize> {
    let texts = |o: &[Arc<Node>]| o.iter().filter_map(|n| if let NodeKind::Text(_) = &n.kind { Some(n.clone()) } else { None }).collect::<Vec<_>>();
    // Each object's pieces of type, and the words of them that no made type holding it has yet.
    let mut objects: Vec<(Vec<Rect>, Vec<String>, bool)> =
        shown.iter().map(|o| (texts(o).iter().filter_map(|n| n.visual_bounds()).collect(), object_words(o), false)).collect();
    let pieces: usize = objects.iter().map(|o| o.0.len()).sum();
    if content.len().saturating_mul(pieces) > MAX_PAIRS {
        return None;
    }
    let holds = |drawn: Rect, r: &Rect| drawn.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(*r).area() >= 0.5 * r.area().max(1e-9);
    for c in content.iter_mut() {
        let Content::Made(node, region, seen) = c else { continue };
        // Hidden made type has no region: where it draws, shown, is its own box.
        let Some(drawn) = region.or_else(|| node.visual_bounds()) else { continue };
        let spelled = if let NodeKind::Text(t) = &node.kind { spelling(&t.plain_text()) } else { String::new() };
        for (rects, words, held) in &mut objects {
            if rects.iter().any(|r| holds(drawn, r)) {
                *held = true;
                *seen |= region.is_some();
                words.retain(|w| !spelled.contains(w.as_str()));
            }
        }
    }
    if !lines {
        return Some(objects.iter().filter(|(_, words, held)| !held || !words.is_empty()).count());
    }
    // The shown made type that holds no piece (set along a line: not on a path, not vertical), with
    // where it draws, the way its lines run and its words as it writes them.
    let mut loose: Vec<(usize, Rect, f64, Vec<String>)> = content
        .iter()
        .enumerate()
        .filter_map(|(at, c)| match c {
            Content::Made(node, Some(drawn), false) => match &node.kind {
                NodeKind::Text(t) if !t.vertical && !matches!(t.kind, TextKind::OnPath { .. }) => {
                    Some((at, *drawn, turn(t), word_pieces(&plain_text(node)).map(str::to_string).collect()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    for (k, (_, words, held)) in objects.iter_mut().enumerate() {
        if words.is_empty() || loose.is_empty() {
            continue;
        }
        // All of the object's words that are left, or none: a piece the text objects only partly
        // account for isn't theirs (the page may be of another state of the file).
        let page = shown.get(k).map_or_else(Vec::new, |o| words_drawn(o));
        let (mut left, mut takers) = (words.clone(), vec![]);
        for (i, (_, drawn, turned, written)) in loose.iter().enumerate() {
            if take_words_drawn_at(&page, &mut left, written, *drawn, *turned) {
                takers.push(i);
            }
        }
        if left.is_empty() && !takers.is_empty() {
            (*words, *held) = (left, true);
            for i in takers.into_iter().rev() {
                let (at, ..) = loose.remove(i);
                if let Some(Content::Made(_, _, seen)) = content.get_mut(at) {
                    *seen = true;
                }
            }
        }
    }
    Some(objects.iter().filter(|(_, words, held)| !held || !words.is_empty()).count())
}

/// The way the lines of `t` run on the document: the angle of its baseline, in radians.
fn turn(t: &vectorcraft_doc::TextObject) -> f64 {
    let [a, b, ..] = t.xf.as_coeffs();
    b.atan2(a)
}

/// Each word of the page's text object `o` as it is written (see [`word_pieces`]) with where the
/// page draws it and the way its line runs ([`turn`]), in the order of its pieces: the glyphs'
/// advances along each line, a line high. Nothing for a piece on a path or set vertically, whose
/// words don't lie along a line this way.
fn words_drawn(o: &[Arc<Node>]) -> Vec<(String, Rect, f64)> {
    let mut out = vec![];
    for n in o {
        let NodeKind::Text(t) = &n.kind else { continue };
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        if layout.vertical || layout.on_path {
            continue;
        }
        let text = t.plain_text();
        let mut at = 0;
        for word in word_pieces(&text) {
            // `word` is a slice of `text` after the words before it.
            let Some(start) = text.get(at..).and_then(|rest| rest.find(word)).map(|i| at + i) else { break };
            at = start + word.len();
            let glyphs = layout.glyphs.iter().filter(|g| (start..at).contains(&g.byte));
            let lines = glyphs.map(|g| {
                let size = t.runs.get(g.run).map_or(0.0, |r| r.style.size);
                Rect::new(g.origin.x, g.origin.y - size, g.origin.x + g.advance, g.origin.y + 0.25 * size)
            });
            if let Some(r) = lines.reduce(|a, b| a.union(b)).filter(|r| r.is_finite()) {
                out.push((word.to_string(), t.xf.transform_rect_bbox(r), turn(t)));
            }
        }
    }
    out
}

/// Take from `left` (the [`spelling`]s of a page object's words that no text object has yet) the
/// words `written` (a made text object's, as it writes them) where `page` (the object's words with
/// where the page draws each, [`words_drawn`]) has them following one another, each still left,
/// written the same (case and marks too), running the way the made type's lines run (`turned`,
/// within about six degrees), and together drawn in about the place `drawn` where the made type
/// draws: each box holds half of the other, or round it as far as strokes reach. Whether they were
/// taken; nothing is otherwise.
fn take_words_drawn_at(page: &[(String, Rect, f64)], left: &mut Vec<String>, written: &[String], drawn: Rect, turned: f64) -> bool {
    if written.is_empty() || written.len() > page.len() {
        return false;
    }
    let half = |a: Rect, b: Rect| a.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(b).area() >= 0.5 * b.area().max(1e-9);
    for run in page.windows(written.len()) {
        // The same way round, whichever of the two turns is the shorter.
        let same_way = |angle: f64| (angle - turned).sin().abs() < 0.1 && (angle - turned).cos() > 0.0;
        if !run.iter().zip(written).all(|((word, _, angle), w)| word == w && same_way(*angle)) {
            continue;
        }
        // Every word of the run is still left, as many times as the run has it.
        let mut rest = left.clone();
        let spelled: Vec<String> = run.iter().map(|(word, ..)| spelling(word)).filter(|w| !w.is_empty()).collect();
        let all_left = spelled.iter().all(|w| rest.iter().position(|l| l == w).map(|i| rest.remove(i)).is_some());
        let Some(place) = run.iter().map(|(_, r, _)| *r).reduce(|a, b| a.union(b)) else { continue };
        if all_left && !spelled.is_empty() && half(drawn, place) && half(place, drawn) {
            *left = rest;
            return true;
        }
    }
    false
}

/// What goes into each of `infos`' slots when the file's text document can't make the type of every
/// slot that shows: type made from the text document (`made`) where it can, else the page's type that
/// the text document puts where the page has it, else the page's type in a copy that a layer's or
/// group's look draws (see [`fill_look_slots`]), else the page's type in painting order. The page's
/// type that lies where made type draws and has its words is that type as the page draws it; the
/// rest goes only into slots without made type.
pub(super) fn plan(
    infos: &[SlotInfo],
    stories: &[Option<Arc<Story>>],
    made: Vec<Option<Made>>,
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
    // A story's lines are its first frame's: a frame of a story whose type is made in another
    // text object can't say that the page's type there is its own.
    let made_stories: std::collections::BTreeSet<u32> = infos.iter().zip(&made).filter(|(_, m)| m.is_some()).filter_map(|(i, _)| i.story).collect();
    let open: Vec<bool> = infos.iter().zip(&made).map(|(i, m)| m.is_none() && i.story.is_none_or(|s| !made_stories.contains(&s))).collect();
    let owner = match_page_objects(infos, &lines, &open, shown);
    // Where the made type that doesn't show lies, by slot.
    let hidden_at: Vec<(usize, Rect)> =
        infos.iter().zip(&made).enumerate().filter_map(|(k, (info, m))| Some((k, m.as_ref().filter(|_| !info.shown)?.1?))).collect();
    // Type the text document makes, shown or not: the page's pieces of it aren't needed. Its box
    // stays that of type without a layout here: with text objects whose type isn't made stacked on
    // made type that has their words (the copies of an outlined heading), the page's pieces in the
    // box where the made type draws are theirs too, and the made type would take them all.
    let mut content = made_content(infos, made, false);
    // Is `object` the type of a text object that doesn't show, as the page draws it: in its frame,
    // with its words? (A copied object's lines are where its original's are, so they don't say.)
    let hidden_type = |object: &[Arc<Node>]| {
        let Some(r) = extent(object) else { return false };
        let texts: Vec<Vec<String>> =
            object.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(words(&t.plain_text())) } else { None }).collect();
        hidden_at.iter().any(|(k, place)| {
            place.intersect(r).area() >= 0.5 * r.area().max(1e-9)
                && lines.get(*k).and_then(Option::as_ref).is_some_and(|l| {
                    !texts.is_empty()
                        && texts.iter().all(|ws| !ws.is_empty() && ws.iter().all(|w| l.iter().any(|(text, _)| letters(text).contains(w.as_str()))))
                })
        })
    };
    // The letters and digits of each slot's made type that shows, in any script, in lower case.
    let spelled: Vec<String> = content
        .iter()
        .map(|c| match c {
            Content::Made(node, Some(_), _) => match &node.kind {
                NodeKind::Text(t) => spelling(&t.plain_text()),
                _ => String::new(),
            },
            _ => String::new(),
        })
        .collect();
    let mut spare: Vec<Vec<Arc<Node>>> = vec![];
    for (j, object) in shown.iter().enumerate() {
        match owner.get(j).copied().flatten() {
            Some(k) => match content.get_mut(k) {
                Some(Content::Page(p)) => p.extend(object.iter().cloned()),
                Some(c) => *c = Content::Page(object.clone()),
                None => {}
            },
            // What lies where made type draws, with its words, is that type as the page draws it (in
            // another font, maybe, and in any script).
            None if covered(object, &content, |_| true) && has_made_words(object, &content, &spelled) => {}
            None if hidden_type(object) => {}
            None => spare.push(object.clone()),
        }
    }
    // The page's type that no made type takes is the spare type below.
    // Not the lines the page draws as one piece: here the page's pieces also go into slots without
    // made type, and a piece given to one would be drawn again by the text objects along it.
    let _ = see_page_type(&mut content, shown, false);
    for (k, c) in content.iter_mut().enumerate() {
        if let (Content::Page(parts), Some(Some(l))) = (c, lines.get(k)) {
            join_pieces(parts, l);
        }
    }
    fill_look_slots(infos, &mut content, &mut spare);
    // What is left of the page goes by painting order into the shown slots the file can't say.
    let loose = readable_first(
        stories,
        infos.iter().enumerate().filter(|(k, i)| i.shown && matches!(content.get(*k), Some(Content::Empty))).map(|(k, _)| k).collect(),
    );
    if loose.is_empty() {
        // Type of the page that no text object says it owns goes with the nearest text object that has
        // type of the page (never into made type); with none, it is left out.
        let mut left_out = false;
        for object in &spare {
            let (centre, near) = (extent(object).map(|r| r.center()), |c: &Content| match c {
                Content::Page(parts) => extent(parts).map(|r| r.center()),
                _ => None,
            });
            let nearest = content
                .iter_mut()
                .filter_map(|c| near(c).zip(centre).map(|(p, q)| ((p.x - q.x).hypot(p.y - q.y), c)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            match nearest {
                Some((_, Content::Page(parts))) => parts.extend(object.iter().cloned()),
                _ => left_out = true,
            }
        }
        if !spare.is_empty() {
            warn.push(
                if left_out {
                    PAGE_TYPE_LEFT_OUT
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
    fill_hidden(infos, stories, &mut content, hidden);
    content
}

/// The slots `open` that are to take the page's type left over, by painting order: those of blank
/// stories (nothing but spaces and line ends) only when no other slot is open, as such a story has
/// no type of its own; the blank ones left out come in empty or not at all (see
/// `settle_blank_stories`).
fn readable_first(stories: &[Option<Arc<Story>>], open: Vec<usize>) -> Vec<usize> {
    let blank = |k: &usize| stories.get(*k).and_then(Option::as_ref).is_some_and(|s| s.is_blank());
    if open.iter().all(blank) { open } else { open.into_iter().filter(|k| !blank(k)).collect() }
}

/// The page's type that doesn't show (`hidden`) goes by painting order into the slots that don't
/// show and have nothing yet ([`readable_first`]), but for what is the type of made type that doesn't show: a page object
/// with words and at least half of it where such type draws, while the type has all its words left
/// (each word an object takes is the type's no longer). Past `MAX_PAIRS` comparisons all of it goes.
fn fill_hidden(infos: &[SlotInfo], stories: &[Option<Arc<Story>>], content: &mut [Content], hidden: &[Vec<Arc<Node>>]) {
    let unread = readable_first(
        stories,
        infos.iter().enumerate().filter(|(k, i)| !i.shown && matches!(content.get(*k), Some(Content::Empty))).map(|(k, _)| k).collect(),
    );
    if unread.is_empty() {
        return;
    }
    // Where each made type that doesn't show draws, and the letters it has left.
    let mut made: Vec<(Rect, String)> = content
        .iter()
        .filter_map(|c| match c {
            Content::Made(node, None, _) => node.visual_bounds().map(|b| (b.inflate(OUTLINE_REACH, OUTLINE_REACH), spelling(&plain_text(node)))),
            _ => None,
        })
        .collect();
    let pairs = made.len().saturating_mul(hidden.len());
    // Is `o` the type of made type that doesn't show? That type's letters for it are taken.
    let mut theirs = |o: &[Arc<Node>]| {
        let words = object_words(o);
        let Some(r) = extent(o).filter(|_| !words.is_empty()) else { return false };
        let found = made.iter_mut().find_map(|(b, s)| {
            if b.intersect(r).area() < 0.5 * r.area().max(1e-9) {
                return None;
            }
            let mut left = s.clone();
            for w in &words {
                let at = left.find(w.as_str())?;
                left.replace_range(at..at + w.len(), "");
            }
            Some((s, left))
        });
        found.map(|(s, left)| *s = left).is_some()
    };
    let hidden: Vec<Vec<Arc<Node>>> = if pairs > MAX_PAIRS { hidden.to_vec() } else { hidden.iter().filter(|o| !theirs(o)).cloned().collect() };
    for (slot, parts) in unread.iter().zip(assign(&hidden, unread.len())) {
        if let Some(c) = content.get_mut(*slot) {
            *c = Content::Page(parts.into_iter().flat_map(|p| p.iter().cloned()).collect());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Rect, take_words_drawn_at};

    /// A made text object takes words of a piece only where they follow one another, are written
    /// as it writes them, are still left and are drawn about where it draws.
    #[test]
    fn made_type_takes_the_words_the_page_draws_where_it_draws() {
        let word = |w: &str, x: f64| (w.to_string(), Rect::new(x, 0.0, x + 20.0, 14.0), 0.0);
        let page = [word("DOWN", 0.0), word("TO", 100.0), word("GO", 125.0), word("20", 300.0)];
        let all = || vec!["down".to_string(), "to".to_string(), "go".to_string(), "20".to_string()];
        let take = |left: &mut Vec<String>, written: &[&str], x0: f64, x1: f64| {
            let written: Vec<String> = written.iter().map(|w| w.to_string()).collect();
            take_words_drawn_at(&page, left, &written, Rect::new(x0, 0.0, x1, 14.0), 0.0)
        };
        let mut left = all();
        assert!(take(&mut left, &["TO", "GO"], 100.0, 145.0));
        assert_eq!(left, ["down", "20"]);
        // Taken once: a copy stacked on it finds them gone.
        assert!(!take(&mut left, &["TO", "GO"], 100.0, 145.0));
        // Drawn elsewhere along the piece, written another way, not next to each other, part of a word.
        for (written, x0, x1) in [
            (&["20"][..], 0.0, 20.0),
            (&["Down"][..], 0.0, 20.0),
            (&["DOWN!"][..], 0.0, 20.0),
            (&["DOWN", "GO"][..], 0.0, 145.0),
            (&["DOW"][..], 0.0, 15.0),
        ] {
            let mut left = all();
            assert!(!take(&mut left, written, x0, x1), "{written:?}");
            assert_eq!(left, all());
        }
        // A word in a box many times its size isn't the word.
        let mut left = all();
        assert!(!take(&mut left, &["20"], 0.0, 320.0));
        // Nor is a word whose line runs another way: a quarter turn, or upside down.
        for turned in [std::f64::consts::FRAC_PI_2, std::f64::consts::PI] {
            let mut left = all();
            assert!(!take_words_drawn_at(&page, &mut left, &["20".to_string()], Rect::new(300.0, 0.0, 320.0, 14.0), turned), "{turned}");
        }
    }
}
