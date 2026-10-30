use super::layers::closed_where_ends_meet;
use super::slots::{Content, SlotInfo, assign, covered, extent, join_pieces, letters, same_type, words};
use super::{Affine, Arc, Node, NodeId, NodeKind, Point, Rect, Story, TextKind};

/// Match page objects to the nearest slot whose story describes them, including hidden slots.
fn match_page_objects(infos: &[SlotInfo], lines: &[Option<Vec<(String, Point)>>], shown: &[Vec<Arc<Node>>]) -> Vec<Option<usize>> {
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

/// What goes into each of `infos`' slots: the page's type that the file's text document puts where the
/// page has it, else type made from the text document, else (for a file whose text document can't say)
/// the page's type in painting order.
pub(super) fn plan(
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
    let owner = match_page_objects(infos, &lines, shown);
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
            frame_text_from_art(info, &mut node, &mut region);
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
    // The letters of each slot's story, in lower case.
    let spelled: Vec<String> = stories.iter().map(|s| s.as_ref().map(|s| letters(s.text()).to_ascii_lowercase()).unwrap_or_default()).collect();
    // For each slot: is its made type where `object` is, with the words of one of `object`'s texts in
    // its story? All are false unless such slots have the words of every text between them.
    let spelling = |object: &[Arc<Node>], content: &[Content]| -> Vec<bool> {
        let r = extent(object);
        let texts: Vec<Vec<String>> = object
            .iter()
            .filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(words(&t.plain_text().to_ascii_lowercase())) } else { None })
            .filter(|ws| !ws.is_empty())
            .collect();
        let has = |k: usize| spelled.get(k).is_some_and(|s| texts.is_empty() || texts.iter().any(|ws| ws.iter().all(|w| s.contains(w.as_str()))));
        let near: Vec<bool> = content
            .iter()
            .enumerate()
            .map(|(k, c)| match (c, r) {
                (Content::Made(_, Some(region), ..), Some(r)) => region.intersect(r).area() > 0.0 && has(k),
                _ => false,
            })
            .collect();
        let every = texts.iter().flatten().all(|w| near.iter().zip(&spelled).any(|(n, s)| *n && s.contains(w.as_str())));
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
            // What lies where made type is and has that type's words is that type as the page draws it.
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
