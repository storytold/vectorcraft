//! Type → Threaded Text: one story flowing through frames in order: area type and type on a path,
//! in any mix. Illustrator links type objects of any shape, as long as their text is in an area or
//! along a path, not at a point (<https://helpx.adobe.com/illustrator/using/manage-text-area.html>).
//!
//! Frames are ordinary text objects holding their slice of the story; the document lists the
//! thread order. After every edit, threads whose frames changed are re-flowed ([`reflow`]).

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, TextKind, TextObject, TextRun};

use super::edit::selected_roots;
use super::typecmd::refresh_bounds;
use super::{CommandSpec, EngineError, Result, Session, bad, checked_id, cmd, finite_numbers, has_doc, has_multi, has_selection, ok, switch_or};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "text.thread.create",
            "Create",
            ["Type", "Threaded Text"],
            None,
            "{} thread the selected area type and type on a path (and closed paths, which become area frames) in stacking order → {ids}",
            has_multi,
            create
        ),
        cmd!(
            "text.thread.add",
            "Thread Text",
            [],
            None,
            "{from, to?, rect?: [x, y, width, height], at?: [x, y], before?} thread area type or type on a path `from` (a click on its out port) into one more frame, put after it in its thread, or before it with `before: true` (a click on its in port: the story starts there and flows on into `from`): closed path, area type or type on a path `to`, a new rectangle `rect`, or a new frame of `from`'s kind, shape and size with its top-left at `at` (give one of the three) → {id}; the two frames are selected",
            has_doc,
            add
        ),
        cmd!(
            "text.thread.releaseSelection",
            "Release Selection",
            ["Type", "Threaded Text"],
            None,
            "{} take the selected frames out of their threads (the story re-flows through the rest)",
            has_thread_selection,
            release
        ),
        cmd!(
            "text.thread.remove",
            "Remove Threading",
            ["Type", "Threaded Text"],
            None,
            "{} break the selected frames' threads; each frame keeps the text it shows",
            has_thread_selection,
            remove
        ),
        cmd!(
            "text.thread.break",
            "Break Thread",
            [],
            None,
            "{id, port: \"in\"|\"out\"} break the thread of area type or type on a path `id` before it (its in port) or after it (its out port), as a double-click on that port does: the story flows into the frames before the break, and the frames after it are left empty (still threaded with each other when two or more)",
            has_doc,
            break_thread
        ),
        cmd!(query "text.thread.info", "Threads", [], None, "{} → [[id…]] every thread in order", has_doc, |s, _| {
            Ok(json!(s.doc()?.doc.text_threads.iter().map(|t| t.iter().map(|i| i.0).collect::<Vec<_>>()).collect::<Vec<_>>()))
        }),
    ]
}

fn has_thread_selection(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    let st = s.active().ok_or("no document open")?;
    if st.selection.objects.iter().any(|id| st.doc.text_threads.iter().any(|t| t.contains(id))) {
        Ok(())
    } else {
        Err("no threaded text selected".into())
    }
}

/// The text object `id` when it can be a frame of a thread: area type or type on a path.
fn text_of(d: &Document, id: NodeId) -> Option<&TextObject> {
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) if !matches!(t.kind, TextKind::Point) => Some(t),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: how many threads' stories [`reflow`] copied on this thread.
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Re-flow every thread with a frame that changed between `before` and `doc` (structural sharing
/// makes "changed" a pointer comparison). Deleted frames' text flows into the remaining ones; a
/// frame that is now point type leaves its thread with the text it holds.
pub(crate) fn reflow(before: &Document, doc: &mut Document) {
    let threads = std::mem::take(&mut doc.text_threads);
    let mut kept = vec![];
    for thread in threads {
        let changed = thread.iter().any(|id| match (before.node(*id), doc.node(*id)) {
            (Some(a), Some(b)) => !std::ptr::eq(a, b),
            (None, None) => false,
            _ => true,
        });
        // When no frame changed, the thread keeps its area-type frames and their text is not copied.
        if !changed {
            let live: Vec<NodeId> = thread.into_iter().filter(|id| text_of(doc, *id).is_some()).collect();
            if live.len() >= 2 {
                kept.push(live);
            }
            continue;
        }
        #[cfg(test)]
        COPIED.with(|c| c.set(c.get() + 1));
        // The story: live frames' runs, plus the text of frames deleted by this edit.
        let mut story: Vec<TextRun> = vec![];
        let mut sources: Vec<TextObject> = vec![];
        let mut later_wins = vec![];
        let mut live = vec![];
        for id in &thread {
            match (text_of(doc, *id), text_of(before, *id)) {
                (Some(t), old) => {
                    story.extend(t.runs.iter().cloned());
                    // A frame whose first paragraph was just restyled gives the paragraph it
                    // shares with the frame before it that style.
                    later_wins.push(old.is_some_and(|o| o.para_at(0) != t.para_at(0)));
                    sources.push(t.clone());
                    live.push(*id);
                }
                // A frame converted to point type keeps the text it holds and leaves the thread.
                (None, Some(_)) if matches!(doc.node(*id).map(|n| &n.kind), Some(NodeKind::Text(_))) => {}
                (None, Some(old)) => {
                    story.extend(old.runs.iter().cloned());
                    later_wins.push(false);
                    sources.push(old.clone());
                }
                (None, None) => {}
            }
        }
        if live.is_empty() {
            continue;
        }
        let source_refs: Vec<&TextObject> = sources.iter().collect();
        let paras = vectorcraft_text::thread::story_paras(&source_refs, &later_wins);
        vectorcraft_text::edit::normalize(&mut story);
        if live.len() >= 2 {
            // Threaded frames keep their size: Auto Size is off for them (as in Illustrator).
            for id in &live {
                if text_of(doc, *id).is_some_and(|t| t.area.fit == vectorcraft_doc::AreaFit::AutoHeight)
                    && let Some(NodeKind::Text(t)) = doc.node_mut(*id).map(|n| &mut n.kind)
                {
                    t.area.fit = vectorcraft_doc::AreaFit::None;
                }
            }
        }
        let optical = if live.len() >= 2 { optical_margin(before, doc, &live) } else { None };
        let frames: Vec<TextObject> = live.iter().filter_map(|id| text_of(doc, *id).cloned()).collect();
        let refs: Vec<&TextObject> = frames.iter().collect();
        let parts = vectorcraft_text::thread::distribute(vectorcraft_text::FontDb::global(), &refs, &story, &paras);
        for (id, part) in live.iter().zip(parts) {
            if let Some(NodeKind::Text(t)) = doc.node_mut(*id).map(|n| &mut n.kind) {
                let mut next = (**t).clone();
                next.runs = part.runs;
                next.set_paragraph_styles(part.paras);
                let optical = optical.unwrap_or(t.optical_margin);
                let continues = part.continues;
                if next.runs != t.runs
                    || next.para != t.para
                    || next.paras != t.paras
                    || optical != t.optical_margin
                    || part.list_count != t.list_count
                    || continues != t.first_para_continues
                {
                    t.runs = next.runs;
                    t.para = next.para;
                    t.paras = next.paras;
                    t.optical_margin = optical;
                    t.list_count = part.list_count;
                    t.first_para_continues = continues;
                    refresh_bounds(t);
                }
            }
        }
        if live.len() >= 2 {
            kept.push(live);
        }
    }
    doc.text_threads = kept;
}

/// Type that is not a frame of a thread is a story of its own: its numbered list paragraphs count
/// from 1. Each such type object with another [`TextObject::list_count`] (a copy of a frame, a
/// frame taken out of its thread, a frame converted to point type, which [`reflow`] takes out of
/// its thread) gets 0 and is laid out again.
pub(crate) fn restart_unthreaded_counts(doc: &mut std::sync::Arc<Document>) {
    let mut stale = vec![];
    doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind
            && t.list_count != 0
        {
            stale.push(n.id);
        }
    });
    if stale.is_empty() {
        return;
    }
    let threaded: std::collections::HashSet<NodeId> = doc.text_threads.iter().flatten().copied().filter(|id| text_of(doc, *id).is_some()).collect();
    stale.retain(|id| !threaded.contains(id));
    if stale.is_empty() {
        return;
    }
    let doc = std::sync::Arc::make_mut(doc);
    for id in stale {
        if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
            t.list_count = 0;
            refresh_bounds(t);
        }
    }
}

/// The Optical Margin Alignment setting that every frame of a thread with the frames `live` takes:
/// that of the first frame whose setting changed between `before` and `doc`, else the first frame's.
fn optical_margin(before: &Document, doc: &Document, live: &[NodeId]) -> Option<bool> {
    let now = |id: &NodeId| text_of(doc, *id).map(|t| t.optical_margin);
    let changed = live.iter().find(|id| text_of(before, **id).is_some_and(|t| Some(t.optical_margin) != now(id)));
    changed.or(live.first()).and_then(now)
}

/// Give every frame of each thread the first frame's Optical Margin Alignment setting, without
/// re-flowing the story: for a document restored from a file as it was saved.
pub(crate) fn share_optical_margin(doc: &mut Document) {
    for thread in doc.text_threads.clone() {
        let live: Vec<NodeId> = thread.into_iter().filter(|id| text_of(doc, *id).is_some()).collect();
        let Some(on) = optical_margin(doc, doc, &live) else { continue };
        for id in &live {
            if let Some(NodeKind::Text(t)) = doc.node_mut(*id).map(|n| &mut n.kind)
                && t.optical_margin != on
            {
                t.optical_margin = on;
                refresh_bounds(t);
            }
        }
    }
}

/// Mark the frames of each thread whose first paragraph continues from the frame before
/// ([`TextObject::first_para_continues`]), without re-flowing the story: for a document restored
/// from a file as it was saved.
pub(crate) fn mark_continued(doc: &mut Document) {
    for thread in doc.text_threads.clone() {
        let live: Vec<NodeId> = thread.into_iter().filter(|id| text_of(doc, *id).is_some()).collect();
        let frames: Vec<&TextObject> = live.iter().filter_map(|id| text_of(doc, *id)).collect();
        let marks = vectorcraft_text::thread::continued(&frames);
        for (id, on) in live.iter().zip(marks) {
            if let Some(NodeKind::Text(t)) = doc.node_mut(*id).map(|n| &mut n.kind)
                && t.first_para_continues != on
            {
                t.first_para_continues = on;
                refresh_bounds(t);
            }
        }
    }
}

/// `ids` followed by the other frames of the threads they are in, each once, in thread order.
pub(crate) fn with_threads(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = ids.to_vec();
    for thread in d.text_threads.iter().filter(|t| t.iter().any(|id| ids.contains(id))) {
        for id in thread {
            if !out.contains(id) && text_of(d, *id).is_some() {
                out.push(*id);
            }
        }
    }
    out
}

/// Every frame of `d`'s threads with its type, for [`leave_threads`] after an edit that may turn
/// some of them into other art.
pub(crate) fn frames(d: &Document) -> Vec<(NodeId, TextObject)> {
    d.text_threads.iter().flatten().filter_map(|id| Some((*id, text_of(d, *id)?.clone()))).collect()
}

/// Update the threads for each frame of `was` (from [`frames`] before the edit) that `d` no longer
/// holds as area type or type on a path under its id. When the art now at that id holds the frame's
/// type under a new id (Expand Appearance groups type with its shadows, outer glows or Crop Marks),
/// that type takes the frame's place in its thread. Otherwise (Create Outlines and Expand Appearance
/// made outlines of it) the frame leaves its thread with the text it shows, and the other frames keep
/// the text they hold. When a thread's last frame leaves, the text it has no room for goes after the
/// text of the last frame that stays. A thread left with one frame is no longer a thread. Each frame
/// that stays continues a paragraph ([`TextObject::first_para_continues`]) only when it is still
/// threaded and the frames before it end inside one. A threaded frame whose mark changes lays out
/// its first line again, so its thread re-flows after the edit.
pub(crate) fn leave_threads(d: &mut Document, was: &[(NodeId, TextObject)]) {
    let gone: Vec<(NodeId, &TextObject, Option<NodeId>)> =
        was.iter().filter(|(id, _)| text_of(d, *id).is_none()).map(|(id, t)| (*id, t, kept_type(d, *id))).collect();
    if gone.is_empty() {
        return;
    }
    let find = |id: &NodeId| gone.iter().find(|(g, _, _)| g == id);
    let mut stayed = vec![];
    for i in 0..d.text_threads.len() {
        let thread = d.text_threads[i].clone();
        if !thread.iter().any(|id| find(id).is_some()) {
            continue;
        }
        let keep: Vec<NodeId> = thread
            .iter()
            .filter_map(|id| match find(id) {
                Some((_, _, kept)) => *kept,
                None => Some(*id),
            })
            .collect();
        if let (Some((_, last, None)), Some(stay)) = (thread.last().and_then(find), keep.last()) {
            take_overflow(d, *stay, last);
        }
        stayed.extend(keep.iter().copied());
        d.text_threads[i] = keep;
    }
    d.text_threads.retain(|t| t.len() >= 2);
    let marks: Vec<(NodeId, bool)> = stayed
        .iter()
        .map(|id| {
            let on = d.text_threads.iter().find(|t| t.contains(id)).is_some_and(|t| {
                let live: Vec<(NodeId, &TextObject)> = t.iter().filter_map(|i| Some((*i, text_of(d, *i)?))).collect();
                let frames: Vec<&TextObject> = live.iter().map(|(_, f)| *f).collect();
                live.iter().zip(vectorcraft_text::thread::continued(&frames)).any(|((i, _), on)| on && i == id)
            });
            (*id, on)
        })
        .collect();
    for (id, on) in marks {
        if text_of(d, id).is_some_and(|t| t.first_para_continues != on)
            && let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind)
        {
            t.first_para_continues = on;
            refresh_bounds(t);
        }
    }
}

/// Cut each frame in `art`, a copy of objects of `d` under their ids, that is the last frame of a
/// thread with frames outside `art`, to the text it shows. When such a frame leaves its thread,
/// [`leave_threads`] puts the text it has no room for after the text of the last frame that stays.
pub(crate) fn keep_shown_text(d: &Document, art: &mut Node) {
    let mut inside = vec![];
    art.walk(&mut |n| inside.push(n.id));
    let last: Vec<NodeId> = d
        .text_threads
        .iter()
        .filter(|t| t.iter().any(|id| !inside.contains(id)))
        .filter_map(|t| t.last().copied().filter(|id| inside.contains(id)))
        .collect();
    if !last.is_empty() {
        cut_to_shown(art, &last);
    }
}

/// Cut the area type and type on a path `ids` in `n` to the text each one shows.
fn cut_to_shown(n: &mut Node, ids: &[NodeId]) {
    if ids.contains(&n.id)
        && let NodeKind::Text(t) = &mut n.kind
        && !matches!(t.kind, TextKind::Point)
    {
        // Split as `take_overflow` splits it: the first part is what the frame shows.
        let from: &TextObject = t;
        let parts = vectorcraft_text::thread::distribute_continuing(
            vectorcraft_text::FontDb::global(),
            &[from, from],
            &from.runs,
            &from.paragraph_styles(),
            from.first_para_continues,
            from.list_count,
        );
        if let (Some(head), Some(_)) = (parts.first(), parts.get(1).filter(|p| p.runs.iter().any(|r| !r.text.is_empty()))) {
            t.runs = head.runs.clone();
            t.set_paragraph_styles(head.paras.clone());
            refresh_bounds(t);
        }
    }
    if let Some(children) = n.children_mut() {
        for c in children {
            cut_to_shown(std::sync::Arc::make_mut(c), ids);
        }
    }
}

/// The only area type or type on a path in the art at `id`.
fn kept_type(d: &Document, id: NodeId) -> Option<NodeId> {
    let mut found = vec![];
    d.node(id)?.walk(&mut |n| {
        if matches!(&n.kind, NodeKind::Text(t) if !matches!(t.kind, TextKind::Point)) {
            found.push(n.id);
        }
    });
    match found[..] {
        [one] => Some(one),
        _ => None,
    }
}

/// Put the text that frame `from` holds beyond what it shows after the text of frame `to`.
fn take_overflow(d: &mut Document, to: NodeId, from: &TextObject) {
    let db = vectorcraft_text::FontDb::global();
    // `from`'s text split as if it flowed on into a second frame: the second part is what `from`
    // has no room for. Its share is measured as it is laid out, continuing a paragraph or not and
    // with its numbered list paragraphs counted on from the frames before it.
    let parts = vectorcraft_text::thread::distribute_continuing(
        db,
        &[from, from],
        &from.runs,
        &from.paragraph_styles(),
        from.first_para_continues,
        from.list_count,
    );
    let Some(rest) = parts.get(1).filter(|p| p.runs.iter().any(|r| !r.text.is_empty())) else { return };
    let mut tail = from.clone();
    tail.runs = rest.runs.clone();
    tail.set_paragraph_styles(rest.paras.clone());
    if let Some(NodeKind::Text(t)) = d.node_mut(to).map(|n| &mut n.kind) {
        let head = (**t).clone();
        t.runs = vectorcraft_text::thread::story(&[&head, &tail]);
        t.set_paragraph_styles(vectorcraft_text::thread::story_paras(&[&head, &tail], &[]));
        refresh_bounds(t);
    }
}

/// A closed path converted to an empty area-type frame with `style`.
fn frame_from_path(n: &Node, style: vectorcraft_doc::CharStyle, system_layout: bool) -> Option<Node> {
    let NodeKind::Path { path, .. } = &n.kind else { return None };
    if !path.subpaths.first().is_some_and(|s| s.closed) {
        return None;
    }
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "", style);
    t.kind = TextKind::Area { frame: path.clone() };
    t.xf = vectorcraft_geom::Affine::IDENTITY;
    t.system_layout = system_layout;
    refresh_bounds(&mut t);
    Some(Node::new(n.id, NodeKind::Text(Box::new(t))))
}

fn create(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "text.thread.create";
    let ids = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let style = ids.iter().find_map(|i| text_of(d, *i)).map(|t| t.first_style()).unwrap_or_default();
    let system = d.setup.system_layout;
    let ok = ids.iter().all(|i| text_of(d, *i).is_some() || d.node(*i).is_some_and(|n| frame_from_path(n, style.clone(), system).is_some()));
    if ids.len() < 2 || !ok || !ids.iter().any(|i| text_of(d, *i).is_some()) {
        return Err(bad(C, "select area type and closed paths (at least one area type object)"));
    }
    let out = ids.clone();
    s.edit("Thread Text", |d, _| {
        for id in &ids {
            if text_of(d, *id).is_none() {
                let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
                let f = frame_from_path(&n, style.clone(), d.setup.system_layout).ok_or_else(|| bad(C, "not a closed path"))?;
                *d.node_mut(*id).ok_or(EngineError::NoNode(*id))? = f;
            }
        }
        // Existing threads containing any of these frames merge into the new one, keeping order.
        let mut order: Vec<NodeId> = vec![];
        for id in &ids {
            match d.text_threads.iter().position(|t| t.contains(id)) {
                Some(i) => {
                    for x in d.text_threads.remove(i) {
                        if !order.contains(&x) {
                            order.push(x);
                        }
                    }
                }
                None if !order.contains(id) => order.push(*id),
                None => {}
            }
        }
        d.text_threads.push(order);
        // Force a re-flow of the new thread on this edit.
        touch(d, ids[0]);
        Ok(())
    })?;
    Ok(json!({ "ids": out.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// `text.thread.add`: the loaded text of a port, placed.
fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.thread.add";
    let given = |k: &str| p.get(k).filter(|v| !v.is_null());
    let from = checked_id(s, given("from").unwrap_or(&Value::Null), C)?;
    let before = switch_or(p, "before", false, C)?;
    let d = &s.doc()?.doc;
    let src = text_of(d, from).ok_or_else(|| bad(C, "`from` must be area type or type on a path"))?;
    let style = src.first_style();
    // The frame to thread into: an existing object (`to`), or a new one beside `from`.
    let (to, new) = match (given("to"), given("rect"), given("at")) {
        (Some(v), None, None) => {
            let to = checked_id(s, v, C)?;
            let parent = d.parent_of(to).and_then(|i| d.node(i));
            let system = d.setup.system_layout;
            let frame = text_of(d, to).is_some() || d.node(to).is_some_and(|n| frame_from_path(n, style.clone(), system).is_some());
            if to == from || !frame || parent.is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })) {
                return Err(bad(C, "`to` must be a closed path, area type or type on a path other than `from`"));
            }
            if d.text_threads.iter().any(|t| t.contains(&to)) {
                return Err(bad(C, "`to` is already threaded"));
            }
            (to, None)
        }
        (None, Some(v), None) => {
            let [x, y, w, h] = finite_numbers(v)
                .filter(|[_, _, w, h]| *w > 0.0 && *h > 0.0)
                .ok_or_else(|| bad(C, "`rect` must be [x, y, width, height] with a positive size"))?;
            let frame = vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, w, h));
            (NodeId(0), Some(empty_frame(src, style.clone(), TextKind::Area { frame }, vectorcraft_geom::Affine::translate((x, y)))))
        }
        (None, None, Some(v)) => {
            let [x, y] = finite_numbers(v).ok_or_else(|| bad(C, "`at` must be [x, y]"))?;
            let shape = match &src.kind {
                TextKind::Area { frame } => frame,
                TextKind::OnPath { path, .. } => path,
                TextKind::Point => return Err(bad(C, "`from` must be area type or type on a path")),
            };
            let b = shape.bounds().map(|b| src.xf.transform_rect_bbox(b)).ok_or_else(|| bad(C, "`from` has an empty frame"))?;
            let xf = vectorcraft_geom::Affine::translate((x - b.x0, y - b.y0)) * src.xf;
            (NodeId(0), Some(empty_frame(src, style.clone(), src.kind.clone(), xf)))
        }
        _ => return Err(bad(C, "give one of `to`, `rect` or `at`")),
    };
    let appearance = d.node(from).map(|n| n.appearance.clone());
    let id = s.edit("Thread Text", |d, sel| {
        let to = match new {
            Some(t) => {
                let (parent, idx, _) = d.position(from).ok_or(EngineError::NoNode(from))?;
                let id = d.alloc_id();
                let mut n = Node::new(id, NodeKind::Text(Box::new(t)));
                if let Some(a) = appearance {
                    n.appearance = a;
                }
                d.insert(parent, idx + 1, n)?;
                id
            }
            None => {
                if text_of(d, to).is_none() {
                    let n = d.node(to).cloned().ok_or(EngineError::NoNode(to))?;
                    *d.node_mut(to).ok_or(EngineError::NoNode(to))? =
                        frame_from_path(&n, style, d.setup.system_layout).ok_or_else(|| bad(C, "not a closed path"))?;
                }
                to
            }
        };
        match d.text_threads.iter_mut().find(|t| t.contains(&from)) {
            Some(t) => {
                let at = t.iter().position(|x| *x == from).map_or(t.len(), |i| if before { i } else { i + 1 });
                t.insert(at, to);
            }
            None if before => d.text_threads.push(vec![to, from]),
            None => d.text_threads.push(vec![from, to]),
        }
        // Force a re-flow of the thread on this edit: the edit leaves existing area type `to`
        // unchanged.
        touch(d, to);
        sel.set([from, to]);
        Ok(to)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// An empty frame of `kind` (area type or type on a path) placed by `xf`, with the type options of
/// `src`.
fn empty_frame(src: &TextObject, style: vectorcraft_doc::CharStyle, kind: TextKind, xf: vectorcraft_geom::Affine) -> TextObject {
    let mut t = src.clone();
    t.runs = vec![TextRun { text: String::new(), style, inline: None }];
    t.set_all_paras(src.para_at(0).clone());
    t.kind = kind;
    t.xf = xf;
    refresh_bounds(&mut t);
    t
}

/// Give a node a fresh allocation so [`reflow`] sees its thread as changed.
fn touch(d: &mut Document, id: NodeId) {
    if let Some(n) = d.node_mut(id) {
        let copy = n.clone();
        *n = copy;
    }
}

fn release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    s.edit("Release Threaded Text", |d, _| {
        // The released frames' text joins the rest of the story (they become empty).
        for thread in d.text_threads.clone() {
            let (out, keep): (Vec<NodeId>, Vec<NodeId>) = thread.iter().partition(|i| ids.contains(i));
            if out.is_empty() {
                continue;
            }
            let mut story: Vec<TextRun> = vec![];
            let sources: Vec<TextObject> = thread.iter().filter_map(|id| text_of(d, *id).cloned()).collect();
            for t in &sources {
                story.extend(t.runs.iter().cloned());
            }
            let paras = vectorcraft_text::thread::story_paras(&sources.iter().collect::<Vec<_>>(), &[]);
            let style = story.first().map(|r| r.style.clone()).unwrap_or_default();
            let first_para = paras.first().cloned().unwrap_or_default();
            for id in &out {
                if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                    t.runs = vec![TextRun { text: String::new(), style: style.clone(), inline: None }];
                    t.set_all_paras(first_para.clone());
                    t.first_para_continues = false;
                    refresh_bounds(t);
                }
            }
            // The first frame that stays holds the whole story, and the others none, for the
            // re-flow to share it out among them once.
            let end_style = story.last().map_or_else(|| style.clone(), |r| r.style.clone());
            let last_para = paras.last().cloned().unwrap_or_default();
            for id in keep.iter().skip(1) {
                if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                    t.runs = vec![TextRun { text: String::new(), style: end_style.clone(), inline: None }];
                    t.set_all_paras(last_para.clone());
                }
            }
            if let Some(first) = keep.first()
                && let Some(NodeKind::Text(t)) = d.node_mut(*first).map(|n| &mut n.kind)
            {
                vectorcraft_text::edit::normalize(&mut story);
                t.runs = story;
                t.set_paragraph_styles(paras);
                // It starts the story, also when it is left alone and so out of a thread.
                t.first_para_continues = false;
                refresh_bounds(t);
            }
            let i = d.text_threads.iter().position(|t| *t == thread).unwrap_or(0);
            d.text_threads[i] = keep;
        }
        d.text_threads.retain(|t| t.len() >= 2);
        Ok(())
    })?;
    ok()
}

fn remove(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    s.edit("Remove Threading", |d, _| {
        let (gone, kept): (Vec<Vec<NodeId>>, Vec<Vec<NodeId>>) =
            std::mem::take(&mut d.text_threads).into_iter().partition(|t| t.iter().any(|i| ids.contains(i)));
        d.text_threads = kept;
        // Each frame's first paragraph now starts in it.
        for id in gone.iter().flatten() {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind)
                && t.first_para_continues
            {
                t.first_para_continues = false;
                refresh_bounds(t);
            }
        }
        Ok(())
    })?;
    ok()
}

/// `text.thread.break`: a double-click on a threaded port. The thread is split there; the whole
/// story moves into the frames before the split, and the frames after it are emptied, staying
/// threaded with each other when two or more remain.
fn break_thread(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.thread.break";
    let id = checked_id(s, p.get("id").unwrap_or(&Value::Null), C)?;
    let before = match p.get("port").and_then(Value::as_str) {
        Some("in") => true,
        Some("out") => false,
        _ => return Err(bad(C, "`port` must be \"in\" or \"out\"")),
    };
    let d = &s.doc()?.doc;
    let thread = d.text_threads.iter().find(|t| t.contains(&id)).cloned().unwrap_or_default();
    let at = thread.iter().position(|x| *x == id).map(|i| if before { i } else { i + 1 });
    let Some(at) = at.filter(|at| *at > 0 && *at < thread.len()) else {
        return Err(bad(C, if before { "no frame comes before `id` in a thread" } else { "no frame comes after `id` in a thread" }));
    };
    let (head, tail) = (thread[..at].to_vec(), thread[at..].to_vec());
    s.edit("Break Thread", |d, _| {
        let sources: Vec<TextObject> = thread.iter().filter_map(|id| text_of(d, *id).cloned()).collect();
        let mut story: Vec<TextRun> = sources.iter().flat_map(|t| t.runs.iter().cloned()).collect();
        let paras = vectorcraft_text::thread::story_paras(&sources.iter().collect::<Vec<_>>(), &[]);
        let end_style = story.last().map(|r| r.style.clone()).unwrap_or_default();
        let last_para = paras.last().cloned().unwrap_or_default();
        // Every frame but the first holds nothing, for the re-flow to share the story out among
        // the first part's frames once.
        for id in head.iter().skip(1).chain(&tail) {
            if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                t.runs = vec![TextRun { text: String::new(), style: end_style.clone(), inline: None }];
                t.set_all_paras(last_para.clone());
                t.first_para_continues = false;
                refresh_bounds(t);
            }
        }
        if let Some(NodeKind::Text(t)) = head.first().and_then(|id| d.node_mut(*id)).map(|n| &mut n.kind) {
            vectorcraft_text::edit::normalize(&mut story);
            t.runs = story;
            t.set_paragraph_styles(paras);
            t.first_para_continues = false;
            refresh_bounds(t);
        }
        if let Some(i) = d.text_threads.iter().position(|t| *t == thread) {
            d.text_threads.splice(i..=i, [head, tail]);
        }
        d.text_threads.retain(|t| t.len() >= 2);
        Ok(())
    })?;
    ok()
}

#[cfg(test)]
mod tests;
