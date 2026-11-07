use super::layers::closed_where_ends_meet;
use super::slots::{
    Content, MAX_PAIRS, OUTLINE_REACH, PAGE_TYPE_LEFT_OUT, PAGE_TYPE_UNCHECKED, SlotInfo, assign, covered, extent, join_pieces, letters, same_type,
    words,
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
    let (mut taller, mut deeper) = (0, 0);
    let made = infos
        .iter()
        .enumerate()
        .map(|(k, info)| {
            let t = template?;
            let story = stories.get(k)?.as_ref()?;
            let mut node = story.node(NodeId(0), info.frame, t, to_doc)?;
            let mut region = story.region(t, to_doc);
            // The first line where the file has it, in the text document's frame (that of a
            // copied object is its original's, as its lines are).
            story.place_first_line(&mut node, info.frame, t, to_doc);
            // Area type and type on a path in the object's own frame: the text document's frame
            // of a copied object keeps its original's path.
            frame_text_from_art(info, &mut node, &mut region);
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
    made
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
pub(super) fn made_only(infos: &[SlotInfo], made: Vec<Option<Made>>, pages: [&[Vec<Arc<Node>>]; 2], warn: &mut Vec<String>) -> Vec<Content> {
    let [shown, hidden] = pages;
    let mut content = made_content(infos, made);
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
    match see_page_type(&mut content, untaken) {
        Some(0) => {}
        Some(_) => warn.push(PAGE_TYPE_LEFT_OUT.into()),
        None => warn.push(PAGE_TYPE_UNCHECKED.into()),
    }
    drop_look_slots(infos, &mut content);
    fill_hidden(infos, &mut content, hidden);
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

/// The words of the page's text object `o`, each as its [`spelling`], the empty ones left out.
fn object_words(o: &[Arc<Node>]) -> Vec<String> {
    o.iter().flat_map(|n| plain_text(n).split_whitespace().map(spelling).collect::<Vec<_>>()).filter(|w| !w.is_empty()).collect()
}

/// Each slot's made type as its content, else nothing yet. Made type that shows has where it draws,
/// or where its story lies when it draws nothing.
fn made_content(infos: &[SlotInfo], made: Vec<Option<Made>>) -> Vec<Content> {
    infos
        .iter()
        .zip(made)
        .map(|(info, m)| match m {
            Some((node, region)) => {
                let drawn = node.visual_bounds().filter(|b| !b.is_zero_area()).or(region);
                Content::Made(node, drawn.filter(|_| info.shown), false)
            }
            None => Content::Empty,
        })
        .collect()
}

/// Mark the made type that has at least half of a piece of the page's type where it draws (or round
/// it, as far as strokes round type reach) as type the page draws. How many of the page's text
/// objects no made type takes, shown or not: none that holds a piece of the object has its words
/// ([`spelling`]). Past `MAX_PAIRS` comparisons nothing is marked or counted (`None`).
fn see_page_type(content: &mut [Content], shown: &[Vec<Arc<Node>>]) -> Option<usize> {
    let texts = |o: &[Arc<Node>]| o.iter().filter_map(|n| if let NodeKind::Text(_) = &n.kind { Some(n.clone()) } else { None }).collect::<Vec<_>>();
    // Each object's pieces of type, and the words of them that no made type holding it has yet.
    let mut objects: Vec<(Vec<Rect>, Vec<String>, bool)> =
        shown.iter().map(|o| (texts(o).iter().filter_map(|n| n.visual_bounds()).collect(), object_words(o), false)).collect();
    let pieces: usize = objects.iter().map(|o| o.0.len()).sum();
    if content.len().saturating_mul(pieces) > MAX_PAIRS {
        return None;
    }
    let holds = |drawn: Rect, r: &Rect| drawn.inflate(OUTLINE_REACH, OUTLINE_REACH).intersect(*r).area() >= 0.5 * r.area().max(1e-9);
    for c in content {
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
    Some(objects.iter().filter(|(_, words, held)| !held || !words.is_empty()).count())
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
    // Type the text document makes, shown or not: the page's pieces of it aren't needed.
    let mut content = made_content(infos, made);
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
    let _ = see_page_type(&mut content, shown);
    for (k, c) in content.iter_mut().enumerate() {
        if let (Content::Page(parts), Some(Some(l))) = (c, lines.get(k)) {
            join_pieces(parts, l);
        }
    }
    fill_look_slots(infos, &mut content, &mut spare);
    // What is left of the page goes by painting order into the shown slots the file can't say.
    let loose: Vec<usize> =
        infos.iter().enumerate().filter(|(k, i)| i.shown && matches!(content.get(*k), Some(Content::Empty))).map(|(k, _)| k).collect();
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
    fill_hidden(infos, &mut content, hidden);
    content
}

/// The page's type that doesn't show (`hidden`) goes by painting order into the slots that don't
/// show and have nothing yet, but for what is the type of made type that doesn't show: a page object
/// with words and at least half of it where such type draws, while the type has all its words left
/// (each word an object takes is the type's no longer). Past `MAX_PAIRS` comparisons all of it goes.
fn fill_hidden(infos: &[SlotInfo], content: &mut [Content], hidden: &[Vec<Arc<Node>>]) {
    let unread: Vec<usize> =
        infos.iter().enumerate().filter(|(k, i)| !i.shown && matches!(content.get(*k), Some(Content::Empty))).map(|(k, _)| k).collect();
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
