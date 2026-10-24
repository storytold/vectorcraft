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
use super::*;

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

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame of this story. ";

    fn setup() -> (Session, u64, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let a = s.execute("text.create", &json!({"x": 10, "y": 10, "text": TEXT.repeat(4), "area": {"width": 150, "height": 60}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        (s, a, b)
    }

    fn text(s: &Session, id: u64) -> String {
        match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.plain_text(),
            _ => panic!("not text"),
        }
    }

    #[test]
    fn thread_flows_reflows_on_edit_and_survives_delete() {
        let (mut s, a, b) = setup();
        s.execute("text.thread.create", &json!({})).unwrap();
        let (ta, tb) = (text(&s, a), text(&s, b));
        assert!(!ta.is_empty() && !tb.is_empty());
        assert_eq!(format!("{ta}{tb}"), TEXT.repeat(4));
        // Editing the first frame re-flows into the second.
        s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "NEW "})).unwrap();
        assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), format!("NEW {}", TEXT.repeat(4)));
        assert_ne!(text(&s, b), tb, "the overflow moved on");
        // Deleting the first frame moves all its text into the second.
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
        assert_eq!(text(&s, b), format!("NEW {}", TEXT.repeat(4)));
        assert!(s.doc().unwrap().doc.text_threads.is_empty(), "a one-frame thread is no thread");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.text_threads.len(), 1);
    }

    #[test]
    fn edits_copy_only_the_stories_of_threads_whose_frames_changed() {
        let (mut s, a, b) = setup();
        s.execute("text.thread.create", &json!({})).unwrap();
        let c = s.execute("text.create", &json!({"x": 10, "y": 200, "text": TEXT.repeat(2), "area": {"width": 150, "height": 40}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let d = s.execute("shape.rectangle", &json!({"x": 200, "y": 220, "width": 150, "height": 60})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [c, d]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        let texts = |s: &Session| [a, b, c, d].map(|id| text(s, id));
        let copied = || COPIED.with(std::cell::Cell::get);
        let (shown, start) = (texts(&s), copied());
        // A rectangle drawn, moved and dragged beside the threads changes none of their frames.
        let r = s.execute("shape.rectangle", &json!({"x": 400, "y": 250, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
        s.begin_interaction("Move").unwrap();
        for dx in 1..=3 {
            s.preview("object.move", &json!({"dx": dx, "dy": 0})).unwrap();
        }
        s.commit_interaction().unwrap();
        assert_eq!(copied(), start, "edits beside the threads copied their stories");
        assert_eq!(texts(&s), shown);
        // Typing in the first thread copies and re-flows that thread's story only.
        s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "NEW "})).unwrap();
        assert_eq!(copied(), start + 1);
        assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), format!("NEW {}", TEXT.repeat(4)));
        assert_eq!(texts(&s)[2..], shown[2..]);
        let ids = |t: [u64; 2]| t.map(NodeId).to_vec();
        assert_eq!(s.doc().unwrap().doc.text_threads, [ids([a, b]), ids([c, d])]);
    }

    /// Time edits of a story of 50,000 characters threaded through 8 frames, each frame wrapping
    /// around an ellipse, with one style, a style change every 10 words and a style change at every
    /// word: 21 characters typed into the first frame, then 200 moves of a rectangle below the
    /// frames and 200 frames of a drag of it.
    /// Run with `cargo test --release -p vectorcraft-engine --lib threaded_story_edit_timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual timing probe; wall-clock performance depends on machine load"]
    fn threaded_story_edit_timing() {
        fn median(s: &mut Session, n: u32, f: &dyn Fn(&mut Session, f64)) -> f64 {
            let mut ms: Vec<f64> = (1..=n)
                .map(|i| {
                    let start = std::time::Instant::now();
                    f(s, f64::from(i));
                    start.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            ms.sort_by(f64::total_cmp);
            ms[ms.len() / 2]
        }
        let story = format!("{}\n", TEXT.repeat(6).trim_end()).repeat(81)[..50_000].to_string();
        for words in [0, 10, 1] {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 4400, "height": 1200})).unwrap();
            let area = json!({"width": 500, "height": 900});
            let first = s.execute("text.create", &json!({"x": 20, "y": 20, "text": story, "size": 9, "area": area})).unwrap()["id"].as_u64().unwrap();
            if words > 0 {
                s.edit("Restyle", |d, _| {
                    let Some(NodeKind::Text(t)) = d.node_mut(NodeId(first)).map(|n| &mut n.kind) else { panic!("not text") };
                    let style = t.runs[0].style.clone();
                    let pieces: Vec<&str> = story.split_inclusive([' ', '\n']).collect();
                    t.runs = pieces
                        .chunks(words)
                        .enumerate()
                        .map(|(i, c)| {
                            let mut run = TextRun::new(c.concat(), style.clone());
                            run.style.underline = i % 2 == 1;
                            run
                        })
                        .collect();
                    refresh_bounds(t);
                    Ok(())
                })
                .unwrap();
            }
            let mut ids = vec![first];
            for k in 1..8 {
                let frame = json!({"x": 20 + k * 540, "y": 20, "width": 500, "height": 900});
                ids.push(s.execute("shape.rectangle", &frame).unwrap()["id"].as_u64().unwrap());
            }
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            s.execute("text.thread.create", &json!({})).unwrap();
            for k in 0..8 {
                let e = s.execute("shape.ellipse", &json!({"x": 195 + k * 540, "y": 395, "width": 150, "height": 150})).unwrap()["id"].clone();
                s.execute("object.textWrap.make", &json!({"ids": [e]})).unwrap();
            }
            let r = s.execute("shape.rectangle", &json!({"x": 20, "y": 1100, "width": 30, "height": 30})).unwrap()["id"].clone();
            s.execute("select.set", &json!({"ids": [r]})).unwrap();
            let typing = median(&mut s, 21, &|s, _| {
                s.execute("text.editRange", &json!({"id": first, "start": 0, "end": 0, "insert": "x"})).unwrap();
            });
            let edit = median(&mut s, 200, &|s, _| {
                s.execute("object.move", &json!({"dx": 0.5, "dy": 0})).unwrap();
            });
            s.begin_interaction("Move").unwrap();
            let frame = median(&mut s, 200, &|s, i| {
                s.preview("object.move", &json!({"dx": i * 0.25, "dy": 0})).unwrap();
            });
            s.commit_interaction().unwrap();
            let d = &s.doc().unwrap().doc;
            let runs: usize = ids.iter().filter_map(|id| text_of(d, NodeId(*id))).map(|t| t.runs.len()).sum();
            println!("{runs} runs in 8 frames: typing {typing:.1} ms, a move below them {edit:.3} ms, a drag frame {frame:.3} ms (medians)");
        }
    }

    fn new_id(v: Value) -> u64 {
        v["id"].as_u64().unwrap()
    }

    fn threads(s: &Session) -> Vec<Vec<u64>> {
        s.doc().unwrap().doc.text_threads.iter().map(|t| t.iter().map(|i| i.0).collect()).collect()
    }

    /// Bounds of area type `id`'s frame on the page.
    fn frame_box(s: &Session, id: u64) -> vectorcraft_geom::Rect {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("not area type") };
        let TextKind::Area { frame } = &t.kind else { panic!("not area type") };
        t.xf.transform_rect_bbox(frame.bounds().unwrap())
    }

    /// #2572: the text loaded from an out port threads into a closed path, into a new frame of the
    /// first one's shape and size at a point, and into a new rectangle, each in one undo step.
    #[test]
    fn add_threads_into_a_path_a_frame_at_a_point_and_a_rectangle() {
        let (mut s, a, b) = setup();
        assert_eq!(new_id(s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap()), b);
        assert_eq!(threads(&s), [[a, b]]);
        assert!(!text(&s, b).is_empty(), "the overflow flowed into the path");
        assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), TEXT.repeat(4));
        assert_eq!(s.doc().unwrap().selection.objects, [NodeId(a), NodeId(b)]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(threads(&s).is_empty());
        assert!(matches!(s.doc().unwrap().doc.node(NodeId(b)).unwrap().kind, NodeKind::Path { .. }), "undo gives the path back");

        let c = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [400, 200]})).unwrap());
        assert_eq!(frame_box(&s, c), vectorcraft_geom::Rect::new(400.0, 200.0, 550.0, 260.0), "the first frame's size, its top-left at the click");
        let d = new_id(s.execute("text.thread.add", &json!({"from": c, "rect": [10, 100, 300, 150]})).unwrap());
        assert_eq!(frame_box(&s, d), vectorcraft_geom::Rect::new(10.0, 100.0, 310.0, 250.0));
        assert_eq!(threads(&s), [[a, c, d]], "each new frame comes after the one it was threaded from");
        assert_eq!(format!("{}{}{}", text(&s, a), text(&s, c), text(&s, d)), TEXT.repeat(4));
        assert!(!text(&s, d).is_empty());
        // A frame threaded from the middle of a thread goes in after it.
        let e = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [0, 0]})).unwrap());
        assert_eq!(threads(&s), [[a, e, c, d]]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(NodeId(e)).is_none());
        assert_eq!(threads(&s), [[a, c, d]]);
    }

    /// #2572: the text loaded from an in port threads back: the new frame or the path clicked comes
    /// before the frame it was loaded from, so the story starts there and flows on into it; in the
    /// middle of a thread the new frame goes in between, each in one undo step.
    #[test]
    fn add_before_threads_the_story_from_the_new_frame_into_the_clicked_one() {
        let (mut s, a, b) = setup();
        let c = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [400, 200], "before": true})).unwrap());
        assert_eq!(threads(&s), [[c, a]]);
        assert_eq!(frame_box(&s, c), vectorcraft_geom::Rect::new(400.0, 200.0, 550.0, 260.0), "the clicked frame's size at the click");
        assert!(TEXT.repeat(4).starts_with(&text(&s, c)) && !text(&s, c).is_empty(), "the story starts in the new frame: {:?}", text(&s, c));
        assert_eq!(format!("{}{}", text(&s, c), text(&s, a)), TEXT.repeat(4));
        assert_eq!(s.doc().unwrap().selection.objects, [NodeId(a), NodeId(c)]);
        // In between the frame and the one before it.
        let d = new_id(s.execute("text.thread.add", &json!({"from": a, "rect": [10, 100, 300, 150], "before": true})).unwrap());
        assert_eq!(threads(&s), [[c, d, a]]);
        assert_eq!(format!("{}{}{}", text(&s, c), text(&s, d), text(&s, a)), TEXT.repeat(4));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(threads(&s), [[c, a]]);
        // A closed path clicked with the in port's text comes first, with the start of the story.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(new_id(s.execute("text.thread.add", &json!({"from": a, "to": b, "before": true})).unwrap()), b);
        assert_eq!(threads(&s), [[b, a]]);
        assert!(TEXT.repeat(4).starts_with(&text(&s, b)) && !text(&s, b).is_empty());
        assert_eq!(format!("{}{}", text(&s, b), text(&s, a)), TEXT.repeat(4));
        // `before: false` is the out port's.
        let e = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [0, 0], "before": false})).unwrap());
        assert_eq!(threads(&s), [[b, a, e]]);
    }

    /// Type on a path along a line beside the area type of [`setup`].
    fn type_on_a_line(s: &mut Session) -> u64 {
        let line = new_id(s.execute("shape.line", &json!({"x1": 10, "y1": 250, "x2": 110, "y2": 250})).unwrap());
        new_id(s.execute("text.createInPath", &json!({"path": line, "mode": "onPath", "text": ""})).unwrap())
    }

    /// Type on a path threads as area type does, in any mix: a story flows from area type along a
    /// path and on into area type, and a new frame threaded from type on a path at a point is type
    /// on a path of the same shape.
    #[test]
    fn type_on_a_path_threads_with_area_type() {
        let (mut s, a, b) = setup();
        let on_path = type_on_a_line(&mut s);
        s.execute("text.thread.add", &json!({"from": a, "to": on_path})).unwrap();
        s.execute("text.thread.add", &json!({"from": on_path, "to": b})).unwrap();
        assert_eq!(threads(&s), [[a, on_path, b]]);
        let shares = [a, on_path, b].map(|id| text(&s, id));
        assert!(shares.iter().all(|t| !t.is_empty()), "each frame takes a share: {shares:?}");
        assert_eq!(shares.concat(), TEXT.repeat(4));

        let c = new_id(s.execute("text.thread.add", &json!({"from": on_path, "at": [300, 250]})).unwrap());
        let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(c)).unwrap().kind else { panic!("not text") };
        assert!(matches!(t.kind, TextKind::OnPath { .. }), "type on a path, as the frame it was threaded from");
        assert_eq!(threads(&s), [[a, on_path, c, b]]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(threads(&s), [[a, on_path, b]]);
    }

    /// Type on a path released from its thread gives its text back to the frame before it.
    #[test]
    fn type_on_a_path_is_released_from_its_thread() {
        let (mut s, a, _) = setup();
        let on_path = type_on_a_line(&mut s);
        s.execute("text.thread.add", &json!({"from": a, "to": on_path})).unwrap();
        assert!(!text(&s, on_path).is_empty());
        s.execute("select.set", &json!({"ids": [on_path]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert!(threads(&s).is_empty());
        assert!(text(&s, on_path).is_empty());
        assert_eq!(text(&s, a), TEXT.repeat(4));
    }

    /// Area type threaded into from an out port takes the overflow at once, before its own text.
    #[test]
    fn add_into_area_type_flows_the_story_into_it() {
        let (mut s, a, _) = setup();
        let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Own text.", "area": {"width": 150, "height": 190}})).unwrap());
        s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
        assert_eq!(threads(&s), [[a, c]]);
        assert_eq!(format!("{}{}", text(&s, a), text(&s, c)), format!("{}Own text.", TEXT.repeat(4)));
        assert!(text(&s, c).len() > "Own text.".len(), "the overflow flowed into the frame: {:?}", text(&s, c));
    }

    #[test]
    fn add_refuses_bad_params_and_leaves_the_document_alone() {
        let (mut s, a, b) = setup();
        let line = new_id(s.execute("shape.line", &json!({"x1": 0, "y1": 280, "x2": 50, "y2": 290})).unwrap());
        let before = s.doc().unwrap().doc.clone();
        for p in [
            json!({"to": b}),
            json!({"from": b, "at": [0, 0]}),
            json!({"from": 9999, "at": [0, 0]}),
            json!({"from": -1, "at": [0, 0]}),
            json!({"from": a}),
            json!({"from": a, "to": b, "at": [0, 0]}),
            json!({"from": a, "to": a}),
            json!({"from": a, "to": line}),
            json!({"from": a, "to": 9999}),
            json!({"from": a, "at": [0]}),
            json!({"from": a, "at": ["1", 2]}),
            json!({"from": a, "rect": [0, 0, -10, 10]}),
            json!({"from": a, "rect": [0, 0, 10, 0]}),
            json!({"from": a, "rect": [0, 0, 10]}),
            json!({"from": a, "rect": "big"}),
            json!({"from": a, "at": [0, 0], "before": "yes"}),
            json!({"from": a, "at": [0, 0], "before": 1}),
        ] {
            assert!(s.execute("text.thread.add", &p).is_err(), "{p} was accepted");
            assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "{p} changed the document");
        }
        s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
        let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 200, "text": "x", "area": {"width": 50, "height": 20}})).unwrap());
        assert!(s.execute("text.thread.add", &json!({"from": c, "to": b})).is_err(), "a frame already threaded");
    }

    /// A frame released from a thread that keeps two or more frames gives its text back to them:
    /// the story flows through them once.
    #[test]
    fn release_from_a_longer_thread_keeps_the_story_once() {
        let (mut s, a, b) = setup();
        let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "", "area": {"width": 150, "height": 40}})).unwrap());
        s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
        s.execute("text.thread.add", &json!({"from": c, "to": b})).unwrap();
        assert_eq!(threads(&s), [[a, c, b]]);
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert_eq!(threads(&s), [[a, b]]);
        assert!(text(&s, c).is_empty());
        assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), TEXT.repeat(4));
    }

    /// A thread of `n` frames (2 or 3), each showing part of the story: area type `a` of [`setup`],
    /// for 3 an area type frame that is empty until it is threaded, then the rectangle `b`.
    fn thread_of(n: usize) -> (Session, Vec<u64>) {
        let (mut s, a, b) = setup();
        let frames = if n == 3 {
            let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "", "area": {"width": 150, "height": 40}})).unwrap());
            s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
            s.execute("text.thread.add", &json!({"from": c, "to": b})).unwrap();
            vec![a, c, b]
        } else {
            s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
            vec![a, b]
        };
        assert_eq!(threads(&s), std::slice::from_ref(&frames));
        assert!(frames.iter().all(|id| !text(&s, *id).is_empty()), "each frame shows part of the story");
        (s, frames)
    }

    /// Convert To Point Type on a frame of threaded text takes it out of its thread with the text
    /// it holds, and the other frames keep the text they hold: the story holds each character once.
    /// A thread left with one frame is no thread. Undo threads the frame again.
    #[test]
    fn a_frame_converted_to_point_type_leaves_its_thread_with_its_text() {
        // Soft wraps of the point type become line breaks, so its characters are compared without
        // white space.
        let squeeze = |t: &str| t.split_whitespace().collect::<String>();
        for n in [2, 3] {
            for k in 0..n {
                let (mut s, frames) = thread_of(n);
                let shown: Vec<String> = frames.iter().map(|id| text(&s, *id)).collect();
                let id = frames[k];
                s.execute("type.convertToPointType", &json!({"ids": [id]})).unwrap();
                let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!("not text") };
                assert!(matches!(t.kind, TextKind::Point), "{n} frames, frame {k}: point type");
                assert_eq!(squeeze(&text(&s, id)), squeeze(&shown[k]), "{n} frames, frame {k}: the point type keeps its text");
                for (f, before) in frames.iter().zip(&shown).filter(|(f, _)| **f != id) {
                    assert_eq!(text(&s, *f), *before, "{n} frames, frame {k}: frame {f} keeps its text");
                }
                let rest: Vec<u64> = frames.iter().copied().filter(|f| *f != id).collect();
                assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{n} frames, frame {k}");
                let all: String = frames.iter().map(|f| text(&s, *f)).collect();
                assert_eq!(squeeze(&all), squeeze(&TEXT.repeat(4)), "{n} frames, frame {k}: each character once");
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(threads(&s), std::slice::from_ref(&frames));
                assert_eq!(frames.iter().map(|f| text(&s, *f)).collect::<Vec<_>>(), shown, "{n} frames, frame {k}: undo");
            }
        }
    }

    #[test]
    fn release_and_remove() {
        let (mut s, a, b) = setup();
        s.execute("text.thread.create", &json!({})).unwrap();
        let tb = text(&s, b);
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("text.thread.remove", &json!({})).unwrap();
        assert_eq!(text(&s, b), tb, "remove keeps what each frame shows");
        assert!(s.doc().unwrap().doc.text_threads.is_empty());
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert_eq!(text(&s, b), "");
        assert_eq!(text(&s, a), TEXT.repeat(4), "the story returns to the remaining frame");
    }

    /// `story` threaded through three area type frames, of 150 by 60, 40 and `last` points, each
    /// showing part of it.
    fn three_frames(story: &str, last: f64) -> (Session, [u64; 3]) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let frame = |s: &mut Session, x: f64, y: f64, h: f64, text: &str| {
            new_id(s.execute("text.create", &json!({"x": x, "y": y, "text": text, "area": {"width": 150, "height": h}})).unwrap())
        };
        let a = frame(&mut s, 10.0, 10.0, 60.0, story);
        let b = frame(&mut s, 10.0, 100.0, 40.0, "");
        let c = frame(&mut s, 200.0, 10.0, last, "");
        s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
        s.execute("text.thread.add", &json!({"from": b, "to": c})).unwrap();
        assert_eq!(threads(&s), [[a, b, c]]);
        assert!([a, b, c].iter().all(|id| !text(&s, *id).is_empty()), "each frame shows part of the story");
        (s, [a, b, c])
    }

    /// Whether the document holds area type or type on a path `id`.
    fn is_frame(s: &Session, id: u64) -> bool {
        text_of(&s.doc().unwrap().doc, NodeId(id)).is_some()
    }

    /// Create Outlines and Object › Expand on frames of threaded text take them out of their
    /// thread with the text they show, which their outlines draw, and the other frames keep the
    /// text they hold: each character is in the document once. A thread left with one frame is no
    /// thread. Undo threads the frames again.
    #[test]
    fn outlined_frames_leave_their_thread_with_their_text() {
        for command in ["type.createOutlines", "object.expand"] {
            for out in [&[0][..], &[1], &[2], &[0, 2]] {
                let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
                let shown = frames.map(|id| text(&s, id));
                assert_eq!(shown.concat(), TEXT.repeat(4));
                let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
                s.execute("select.set", &json!({"ids": ids})).unwrap();
                s.execute(command, &json!({})).unwrap();
                let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
                for id in &ids {
                    assert!(!is_frame(&s, *id), "{command} {out:?}: frame {id} is outlined");
                }
                let st = s.doc().unwrap();
                let groups = st.selection.objects.iter().filter(|g| matches!(st.doc.node(**g).map(|n| &n.kind), Some(NodeKind::Group { .. })));
                assert_eq!(groups.count(), ids.len(), "{command} {out:?}: the outlines are selected");
                for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                    assert_eq!(text(&s, *id), *before, "{command} {out:?}: frame {id} keeps its text");
                }
                assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{command} {out:?}");
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(threads(&s), [frames], "{command} {out:?}: undo");
                assert_eq!(frames.map(|id| text(&s, id)), shown, "{command} {out:?}: undo");
            }
        }
    }

    /// Expand Appearance outlines a frame with an effect, and the frame leaves its thread with the
    /// text it shows; Object › Expand of such a frame expands its appearance first.
    #[test]
    fn a_frame_whose_appearance_expands_leaves_its_thread_with_its_text() {
        for command in ["effect.expandAppearance", "object.expand"] {
            let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(4), 200.0);
            s.execute("effect.apply", &json!({"effect": "distort.roughen", "ids": [b]})).unwrap();
            let shown = [a, b, c].map(|id| text(&s, id));
            s.execute("select.set", &json!({"ids": [b]})).unwrap();
            s.execute(command, &json!({})).unwrap();
            assert!(!is_frame(&s, b), "{command}: the frame is outlined");
            assert_eq!((text(&s, a), text(&s, c)), (shown[0].clone(), shown[2].clone()), "{command}: the other frames keep their text");
            assert_eq!(threads(&s), [[a, c]], "{command}");
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(threads(&s), [[a, b, c]], "{command}: undo");
            assert_eq!([a, b, c].map(|id| text(&s, id)), shown, "{command}: undo");
        }
    }

    /// Object › Flatten Transparency outlines frames of threaded text with Convert All Text to
    /// Outlines, and turns transparent frames into flat regions or an image: each such frame leaves
    /// its thread with the text it shows, and the other frames keep the text they hold. Undo threads
    /// the frames again. When the last frame is flattened, the frame before it takes the text it has
    /// no room for.
    #[test]
    fn flattened_frames_leave_their_thread_with_their_text() {
        let cases = [(json!({"textToOutlines": true}), false), (json!({}), true), (json!({"balance": 0}), true)];
        for (options, transparent) in cases {
            for out in [&[0][..], &[1], &[2], &[0, 2]] {
                let case = format!("{options} {out:?}");
                let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
                let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
                if transparent {
                    s.execute("transparency.set", &json!({"ids": ids, "opacity": 50})).unwrap();
                }
                let shown = frames.map(|id| text(&s, id));
                assert_eq!(shown.concat(), TEXT.repeat(4));
                let mut p = options.clone();
                p["ids"] = json!(ids);
                s.execute("object.flattenTransparency", &p).unwrap();
                let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
                for id in &ids {
                    assert!(!is_frame(&s, *id), "{case}: frame {id} is flattened");
                }
                for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                    assert_eq!(text(&s, *id), *before, "{case}: frame {id} keeps its text");
                }
                assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{case}");
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(threads(&s), [frames], "{case}: undo");
                assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
            }
        }
        let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
        let shown = [a, b, c].map(|id| text(&s, id));
        let laid = {
            let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
        };
        assert!(laid.overflow, "the last frame has more text than it shows");
        let end = laid.lines.last().map_or(0, |l| l.end);
        let hidden = shown[2][end..].trim_start_matches(' ');
        s.execute("object.flattenTransparency", &json!({"ids": [c], "textToOutlines": true})).unwrap();
        assert_eq!(text(&s, a), shown[0]);
        assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "the frame before it keeps its text and takes the rest");
        assert_eq!(threads(&s), [[a, b]]);
    }

    /// Expand Appearance keeps type whose only effect is a shadow or Crop Marks as type under a new
    /// id, in a group under the frame's id: the type takes the frame's place in its thread, and
    /// each frame keeps its text, the last frame the text it has no room for too. Type in a group
    /// with Crop Marks keeps its id and stays in its thread.
    #[test]
    fn type_that_expand_appearance_keeps_takes_its_frames_place() {
        for (effect, grouped) in [("stylize.dropShadow", false), ("cropMarks", false), ("cropMarks", true)] {
            for k in [1, 2] {
                let case = format!("{effect} grouped {grouped} frame {k}");
                let (mut s, frames) = three_frames(&TEXT.repeat(12), 100.0);
                let id = frames[k];
                s.execute("select.set", &json!({"ids": [id]})).unwrap();
                let target = if grouped { new_id(s.execute("object.group", &json!({})).unwrap()) } else { id };
                s.execute("effect.apply", &json!({"effect": effect, "item": null, "ids": [target]})).unwrap();
                let shown = frames.map(|id| text(&s, id));
                s.execute("select.set", &json!({"ids": [target]})).unwrap();
                s.execute("effect.expandAppearance", &json!({})).unwrap();
                let mut types = vec![];
                if let Some(n) = s.doc().unwrap().doc.node(NodeId(target)) {
                    n.walk(&mut |m| {
                        if matches!(m.kind, NodeKind::Text(_)) {
                            types.push(m.id.0);
                        }
                    });
                }
                let [kept] = types[..] else { panic!("{case}: the type is kept: {types:?}") };
                assert_eq!(kept == id, grouped, "{case}: the type has a new id unless it was in a group");
                let mut now = frames;
                now[k] = kept;
                assert_eq!(threads(&s), [now], "{case}: the type takes the frame's place");
                assert_eq!(now.map(|id| text(&s, id)), shown, "{case}: each frame keeps its text");
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(threads(&s), [frames], "{case}: undo");
                assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
            }
        }
    }

    /// The text that does not fit a thread's last frame, which its outlines do not draw, stays in
    /// the story: when the last frame is outlined, the frame before it takes that text after its
    /// own.
    #[test]
    fn outlining_the_last_frame_keeps_the_text_it_has_no_room_for() {
        let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
        let shown = [a, b, c].map(|id| text(&s, id));
        let laid = {
            let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
        };
        assert!(laid.overflow, "the last frame has more text than it shows");
        let end = laid.lines.last().map_or(0, |l| l.end);
        let hidden = shown[2][end..].trim_start_matches(' ');
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        s.execute("type.createOutlines", &json!({})).unwrap();
        assert_eq!(text(&s, a), shown[0]);
        assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "the frame before it keeps its text and takes the rest");
        assert_eq!(threads(&s), [[a, b]]);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!([a, b, c].map(|id| text(&s, id)), shown);
    }

    /// Object › Rasterize on frames of threaded text takes them out of their thread with the text
    /// they show, which the image draws, and the other frames keep the text they hold. Undo threads
    /// the frames again. When the last frame is rasterized, the frame before it takes the text it
    /// has no room for.
    #[test]
    fn rasterized_frames_leave_their_thread_with_their_text() {
        for clip in [false, true] {
            for out in [&[0][..], &[1], &[2], &[0, 2]] {
                let case = format!("clippingMask {clip} {out:?}");
                let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
                let shown = frames.map(|id| text(&s, id));
                assert_eq!(shown.concat(), TEXT.repeat(4));
                let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
                s.execute("select.set", &json!({"ids": ids})).unwrap();
                s.execute("object.rasterize", &json!({"ppi": 36, "clippingMask": clip})).unwrap();
                let rest: Vec<u64> = frames.iter().copied().filter(|id| !ids.contains(id)).collect();
                for id in &ids {
                    assert!(!is_frame(&s, *id), "{case}: frame {id} is rasterized");
                }
                for (id, before) in frames.iter().zip(&shown).filter(|(id, _)| rest.contains(id)) {
                    assert_eq!(text(&s, *id), *before, "{case}: frame {id} keeps its text");
                }
                assert_eq!(threads(&s), if rest.len() >= 2 { vec![rest] } else { vec![] }, "{case}");
                s.execute("edit.undo", &json!({})).unwrap();
                assert_eq!(threads(&s), [frames], "{case}: undo");
                assert_eq!(frames.map(|id| text(&s, id)), shown, "{case}: undo");
            }
        }
        let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
        let shown = [a, b, c].map(|id| text(&s, id));
        let laid = {
            let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
        };
        assert!(laid.overflow, "the last frame has more text than it shows");
        let end = laid.lines.last().map_or(0, |l| l.end);
        let hidden = shown[2][end..].trim_start_matches(' ');
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        s.execute("object.rasterize", &json!({"ppi": 36})).unwrap();
        assert_eq!(text(&s, a), shown[0]);
        assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]), "the frame before it keeps its text and takes the rest");
        assert_eq!(threads(&s), [[a, b]]);
    }

    /// Frame `id`'s first line: whether its first paragraph continues from the frame before, where
    /// its text starts from the frame's left edge, and how many list markers it starts with.
    fn first_line(s: &Session, id: u64) -> (bool, f64, usize) {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("not area type") };
        let TextKind::Area { frame } = &t.kind else { panic!("not area type") };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let l = &lay.lines[0];
        let x = lay.glyphs[l.glyph_start..l.glyph_end].iter().find(|g| g.len > 0).map_or(f64::NAN, |g| g.origin.x);
        let markers = lay.markers.iter().filter(|m| lay.glyphs[m.glyphs.start].line == 0).count();
        (t.first_para_continues, (x - frame.bounds().unwrap().x0 - 1e-9).round(), markers)
    }

    /// A paragraph's first-line indent and bullet belong to its first line, in the frame it starts
    /// in: the frame it continues into starts at its left indent, without the bullet. Re-flowing,
    /// saving and opening keep it. In a frame that is no longer threaded and in a copy of a frame,
    /// the first paragraph has its first-line indent and bullet.
    #[test]
    fn a_paragraph_continued_into_the_next_frame_starts_it_without_its_first_line_indent_or_bullet() {
        let (mut s, a, b) = setup();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [a, b], "list": "bullet"})).unwrap();
        assert_eq!((first_line(&s, a), first_line(&s, b)), ((false, 18.0, 1), (true, 18.0, 0)));
        s.execute("text.setFormat", &json!({"ids": [a, b], "list": "none", "leftIndent": 0, "firstLineIndent": 24})).unwrap();
        assert_eq!((first_line(&s, a), first_line(&s, b)), ((false, 24.0, 0), (true, 0.0, 0)));
        assert_eq!(format!("{}{}", text(&s, a), text(&s, b)), TEXT.repeat(4));
        // Typing re-flows the story and keeps it.
        s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "NEW "})).unwrap();
        assert_eq!(first_line(&s, b), (true, 0.0, 0));
        // Saved and opened: a native file re-flows its threads, a restored document keeps its text.
        for name in ["thread.vectorcraft", "thread.eps"] {
            let bytes = if name.ends_with(".eps") {
                let r = s.execute("document.exportEps", &json!({})).unwrap();
                vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
            } else {
                vectorcraft_format::save(&s.doc().unwrap().doc, false)
            };
            let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
            assert_eq!(r["restored"], name.ends_with(".eps"), "{r}");
            assert_eq!(threads(&s), [[a, b]], "{name}");
            assert_eq!((first_line(&s, a), first_line(&s, b)), ((false, 24.0, 0), (true, 0.0, 0)), "{name}");
        }
        // A copy of the second frame is in no thread, and starts with the indent.
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        let copy = s.execute("edit.duplicate", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        assert_eq!((first_line(&s, b), first_line(&s, copy)), ((true, 0.0, 0), (false, 24.0, 0)));
        // …and so is a pasted one, and each copy's cached bounds and baselines are its own layout's.
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("edit.copy", &json!({})).unwrap();
        let pasted = s.execute("edit.paste", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let dragged = s.execute("layer.move", &json!({"ids": [b], "target": a, "place": "above", "copy": true})).unwrap()["ids"][0].as_u64().unwrap();
        assert_eq!((first_line(&s, pasted), first_line(&s, dragged)), ((false, 24.0, 0), (false, 24.0, 0)));
        // Every text object in the document has the cached bounds and baselines of its own layout.
        let own_caches = |s: &Session, what: &str| {
            let mut texts = vec![];
            s.doc().unwrap().doc.walk(|n| {
                if let NodeKind::Text(t) = &n.kind {
                    texts.push((n.id, t));
                }
            });
            for (id, t) in texts {
                let mut fresh = (**t).clone();
                refresh_bounds(&mut fresh);
                assert!(t.cached_bounds.is_some() && t.cached_bounds == fresh.cached_bounds, "{what}: {id:?}");
                assert_eq!(t.cached_baselines, fresh.cached_baselines, "{what}: {id:?}");
            }
        };
        own_caches(&s, "copies");
        // Release to Layers (Build), Release Opacity Mask, Edit Opacity Mask and Draw Inside copy the
        // frame too.
        let layer = s.doc().unwrap().doc.layer_containing(NodeId(b)).unwrap().0;
        s.execute("layer.releaseToLayers", &json!({"id": layer, "build": true})).unwrap();
        own_caches(&s, "Release to Layers (Build)");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({"ids": [a, b]})).unwrap();
        s.execute("transparency.releaseOpacityMask", &json!({})).unwrap();
        own_caches(&s, "Release Opacity Mask");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("transparency.editOpacityMask", &json!({"id": a})).unwrap();
        own_caches(&s, "Edit Opacity Mask");
        s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        for _ in 0..3 {
            s.execute("edit.undo", &json!({})).unwrap();
        }
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("view.drawMode", &json!({"mode": "inside"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 210, "y": 20, "width": 20, "height": 20})).unwrap();
        own_caches(&s, "Draw Inside");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("view.drawMode", &json!({"mode": "normal"})).unwrap();
        assert_eq!((threads(&s), first_line(&s, b)), (vec![vec![a, b]], (true, 0.0, 0)));
        // Remove Threading: each frame keeps its text, and its first paragraph starts in it.
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("text.thread.remove", &json!({})).unwrap();
        assert_eq!(first_line(&s, b), (false, 24.0, 0));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(first_line(&s, b), (true, 0.0, 0));
        // Released, the frame is empty and starts the next text typed into it as a paragraph.
        let continues = |s: &Session, id: u64| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t.first_para_continues,
            _ => panic!("not text"),
        };
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert!(text(&s, b).is_empty() && !continues(&s, b));
        s.execute("edit.undo", &json!({})).unwrap();
        // With the first frame released, the second one holds the whole story and starts it.
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert!(threads(&s).is_empty() && text(&s, a).is_empty());
        assert_eq!(first_line(&s, b), (false, 24.0, 0));
        s.execute("edit.undo", &json!({})).unwrap();
        // Converted to point type, it is in no thread, and its first line starts at the indent where
        // the frame's left edge was.
        let left = frame_box(&s, b).x0;
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("type.convertToPointType", &json!({})).unwrap();
        assert!(!continues(&s, b));
        {
            let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(b)).unwrap().kind else { panic!("not text") };
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            let g = lay.glyphs.iter().find(|g| g.len > 0).unwrap();
            let x = (t.xf * g.origin).x - left;
            assert!((x - 24.0).abs() < 1e-6, "{x}");
        }
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(first_line(&s, b), (true, 0.0, 0));
        // Text that ends the first frame with a paragraph break: the second frame starts a
        // paragraph, with its indent.
        let all = text(&s, a).len();
        s.execute("text.editRange", &json!({"id": a, "start": 0, "end": all, "insert": "Short line.\n".repeat(12)})).unwrap();
        assert!(text(&s, a).ends_with('\n'), "{:?}", text(&s, a));
        assert_eq!(first_line(&s, b), (false, 24.0, 0));
    }

    /// When Create Outlines takes frames out of a thread, a frame that now starts its thread or is
    /// in no thread starts its paragraph with the first-line indent; a frame after one that ends
    /// inside the paragraph still continues it. The frames that stay hold each character they held
    /// once, and keep their text when no mark changes. Undo marks the frames again.
    #[test]
    fn a_frame_that_starts_its_thread_after_outlining_has_its_first_line_indent() {
        for (out, want) in [
            (&[0][..], [None, Some(false), Some(true)]),
            (&[0, 2], [None, Some(false), None]),
            (&[1], [Some(false), None, Some(true)]),
            (&[2], [Some(false), Some(true), None]),
        ] {
            let (mut s, frames) = three_frames(&TEXT.repeat(4), 200.0);
            s.execute("text.setFormat", &json!({"ids": frames, "leftIndent": 0, "firstLineIndent": 24})).unwrap();
            let before = frames.map(|id| first_line(&s, id));
            assert_eq!(before, [(false, 24.0, 0), (true, 0.0, 0), (true, 0.0, 0)], "{out:?}");
            let shown = frames.map(|id| text(&s, id));
            assert_eq!(shown.concat(), TEXT.repeat(4), "{out:?}: the last frame has room for the rest");
            let ids: Vec<u64> = out.iter().map(|k| frames[*k]).collect();
            s.execute("select.set", &json!({"ids": ids})).unwrap();
            s.execute("type.createOutlines", &json!({})).unwrap();
            let stay: Vec<usize> = (0..3).filter(|k| want[*k].is_some()).collect();
            for k in &stay {
                let on = want[*k] == Some(true);
                let indent = if on { 0.0 } else { 24.0 };
                assert_eq!(first_line(&s, frames[*k]), (on, indent, 0), "{out:?}: frame {k}");
            }
            let held: String = stay.iter().map(|k| shown[*k].as_str()).collect();
            assert_eq!(stay.iter().map(|k| text(&s, frames[*k])).collect::<String>(), held, "{out:?}: each character once");
            if stay.iter().all(|k| want[*k] == Some(before[*k].0)) {
                for k in &stay {
                    assert_eq!(text(&s, frames[*k]), shown[*k], "{out:?}: frame {k} keeps its text");
                }
            }
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(frames.map(|id| first_line(&s, id)), before, "{out:?}: undo");
        }
    }

    /// The text that a thread's last frame has no room for is measured as the frame lays it out,
    /// without the first-line indent of the paragraph it continues: when the frame is outlined, the
    /// frame before it takes only that text.
    #[test]
    fn outlining_a_continued_last_frame_keeps_only_the_text_it_has_no_room_for() {
        let (mut s, [a, b, c]) = three_frames(&TEXT.repeat(12), 100.0);
        s.execute("text.setFormat", &json!({"ids": [a, b, c], "leftIndent": 0, "firstLineIndent": 72})).unwrap();
        assert_eq!(first_line(&s, c), (true, 0.0, 0));
        let shown = [a, b, c].map(|id| text(&s, id));
        let laid = {
            let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)) else { panic!("not area type") };
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
        };
        assert!(laid.overflow, "the last frame has more text than it shows");
        let end = laid.lines.last().map_or(0, |l| l.end);
        let hidden = shown[2][end..].trim_start_matches(' ');
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        s.execute("type.createOutlines", &json!({})).unwrap();
        assert_eq!(text(&s, b), format!("{}{hidden}", shown[1]));
    }

    /// The text that a thread's last frame has no room for is measured with the frame's numbered
    /// paragraphs counted on from the frames before it: when the frame is outlined, the frame before
    /// it takes only that text.
    #[test]
    fn outlining_a_numbered_last_frame_keeps_only_the_text_it_has_no_room_for() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let items: String = (1..=7).map(|i| format!("Item {i}\n")).collect();
        let story = format!("{items}{}", TEXT.repeat(4));
        let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 106}})).unwrap());
        let c = new_id(s.execute("text.create", &json!({"x": 200, "y": 10, "text": "", "area": {"width": 150, "height": 20}})).unwrap());
        s.execute("text.thread.add", &json!({"from": a, "to": c})).unwrap();
        // A hanging indent that "I." fits and "VIII." does not: the text after "VIII." starts at
        // the next tab stop.
        s.execute("text.setFormat", &json!({"ids": [a, c], "number": "upperRoman", "leftIndent": 14, "firstLineIndent": -14})).unwrap();
        let shown = [a, c].map(|id| text(&s, id));
        assert!(shown[0].ends_with("Item 7\n") && shown[1].starts_with("The quick"), "{shown:?}");
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(c)).cloned() else { panic!("not area type") };
        assert_eq!(t.list_count, 7);
        let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        assert_eq!(laid.markers.first().map(|m| m.text.as_str()), Some("VIII."));
        assert!(laid.overflow, "the last frame has more text than it shows");
        let end = laid.lines.last().map_or(0, |l| l.end);
        let from_one = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &TextObject { list_count: 0, ..t });
        assert_ne!(from_one.lines.last().map_or(0, |l| l.end), end, "counted from 1, the frame would show other text");
        let hidden = shown[1][end..].trim_start_matches(' ');
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        s.execute("type.createOutlines", &json!({})).unwrap();
        assert_eq!(text(&s, a), format!("{}{hidden}", shown[0]));
    }

    /// A story of one-line paragraphs that each start with a quotation mark, threaded from area
    /// type `a` into the rectangle `b`, which shows the paragraphs that `a` has no room for.
    fn quoted_thread() -> (Session, u64, u64) {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let story = "“Quoted.”\n".repeat(20);
        let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 60}})).unwrap());
        let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        assert!(text(&s, b).starts_with('“'));
        (s, a, b)
    }

    /// Frame `id`'s Optical Margin Alignment setting, and whether its glyphs start left of its frame
    /// (its lines' opening quotation marks hang).
    fn optical(s: &Session, id: u64) -> (bool, bool) {
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("not area type") };
        let TextKind::Area { frame } = &t.kind else { panic!("not area type") };
        (t.optical_margin, t.cached_bounds.unwrap().x0 < frame.bounds().unwrap().x0 - 1.0)
    }

    /// Optical Margin Alignment is a setting of the whole thread: Type › Optical Margin Alignment
    /// in any frame sets it in every frame of the thread, in one undo step, and the lines in every
    /// frame hang. Typing keeps it, and frames taken out of the thread keep it.
    #[test]
    fn optical_margin_alignment_is_set_for_every_frame_of_a_thread() {
        let (mut s, a, b) = quoted_thread();
        assert_eq!((optical(&s, a), optical(&s, b)), ((false, false), (false, false)));
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        let r = s.execute("type.opticalMarginAlignment", &json!({})).unwrap();
        assert_eq!(r, json!({"on": true, "ids": [a, b]}));
        assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)), "the second frame's lines hang too");
        // Turned off from the second frame, it is off in both, and one undo turns it back on in both.
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        assert_eq!(s.execute("type.opticalMarginAlignment", &json!({})).unwrap()["on"], false);
        assert_eq!((optical(&s, a), optical(&s, b)), ((false, false), (false, false)));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
        // Typing in the first frame re-flows the story and keeps the setting.
        s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "“New.”\n"})).unwrap();
        assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
        // Any edit that sets one frame sets the thread.
        s.edit("Set", |d, _| {
            let Some(NodeKind::Text(t)) = d.node_mut(NodeId(b)).map(|n| &mut n.kind) else { panic!("not text") };
            t.optical_margin = false;
            refresh_bounds(t);
            Ok(())
        })
        .unwrap();
        assert_eq!((optical(&s, a), optical(&s, b)), ((false, false), (false, false)));
        s.execute("edit.undo", &json!({})).unwrap();
        // Released or unthreaded, each frame keeps it.
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert_eq!((optical(&s, a).0, optical(&s, b).0), (true, true));
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("text.thread.remove", &json!({})).unwrap();
        assert!(threads(&s).is_empty());
        assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
    }

    /// Threading type into a thread gives the joined thread the setting of its first frame.
    #[test]
    fn a_thread_takes_its_first_frames_optical_margin_alignment() {
        let (mut s, a, b) = quoted_thread();
        s.execute("text.thread.remove", &json!({})).unwrap();
        s.execute("type.opticalMarginAlignment", &json!({"ids": [a], "on": true})).unwrap();
        assert_eq!((optical(&s, a).0, optical(&s, b).0), (true, false));
        // Threaded again in stacking order: the second frame takes the first one's setting.
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        assert_eq!((optical(&s, a), optical(&s, b)), ((true, true), (true, true)));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!((optical(&s, a).0, optical(&s, b).0), (true, false));
        // From `b`'s out port into `a`: `a` joins `b`'s thread after it and takes its setting.
        s.execute("text.thread.add", &json!({"from": b, "to": a})).unwrap();
        assert_eq!(threads(&s), [[b, a]]);
        assert_eq!((optical(&s, a).0, optical(&s, b).0), (false, false));
        // A new frame threaded from a frame takes the thread's setting.
        s.execute("type.opticalMarginAlignment", &json!({"ids": [a], "on": true})).unwrap();
        let c = new_id(s.execute("text.thread.add", &json!({"from": a, "at": [400, 10]})).unwrap());
        assert_eq!([a, b, c].map(|id| optical(&s, id).0), [true; 3]);
    }

    /// A file saved with the frames of one thread set differently opens with the first frame's
    /// setting in every frame: a native file, and an EPS file restored from the document it carries.
    #[test]
    fn a_thread_saved_with_differing_frames_opens_with_the_first_frames_setting() {
        let (mut s, a, b) = quoted_thread();
        for (first, name) in [(true, "old.vectorcraft"), (false, "old.vectorcraft"), (true, "old.eps"), (false, "old.eps")] {
            for (id, on) in [(a, first), (b, !first)] {
                let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
                let Some(NodeKind::Text(t)) = doc.node_mut(NodeId(id)).map(|n| &mut n.kind) else { panic!("not text") };
                t.optical_margin = on;
            }
            let bytes = if name.ends_with(".eps") {
                let r = s.execute("document.exportEps", &json!({})).unwrap();
                vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
            } else {
                vectorcraft_format::save(&s.doc().unwrap().doc, false)
            };
            let r = s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes)})).unwrap();
            assert_eq!(r["restored"], name.ends_with(".eps"), "{r}");
            assert_eq!(threads(&s), [[a, b]], "{name}");
            assert_eq!((optical(&s, a), optical(&s, b)), ((first, first), (first, first)), "{name}, first frame {first}");
        }
    }

    /// One count runs through a thread: the second frame's numbered paragraphs go on from the
    /// first frame's, also after an edit, a save and a reopen. A pasted or duplicated copy of a
    /// frame, a frame converted to point type, the frames of a thread that Remove Threading breaks
    /// and the frame a Release Selection leaves alone count from 1.
    #[test]
    fn numbered_list_paragraphs_count_on_through_a_thread() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let items = (1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
        let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": items, "area": {"width": 150, "height": 60}})).unwrap());
        let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
        let numbers = |s: &Session, id: u64| {
            let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| &n.kind) else { panic!("text") };
            vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
        };
        let all = |from: usize| (from..=10).map(|n| format!("{n}.")).collect::<Vec<_>>();
        let first = numbers(&s, a);
        assert!((2..10).contains(&first.len()), "both frames hold items: {first:?}");
        assert_eq!([first.clone(), numbers(&s, b)].concat(), all(1));
        // Typing a paragraph into the first frame numbers it and moves the rest on.
        s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "Item 0\n"})).unwrap();
        assert_eq!([numbers(&s, a), numbers(&s, b)].concat(), [vec!["1.".to_string()], all(2), vec!["11.".to_string()]].concat());
        s.execute("edit.undo", &json!({})).unwrap();
        let reopened = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        let Some(NodeKind::Text(back)) = reopened.node(NodeId(b)).map(|n| &n.kind) else { panic!("text") };
        let after: Vec<String> = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), back).markers.iter().map(|m| m.text.clone()).collect();
        assert_eq!(after, all(first.len() + 1), "reopened");
        // A pasted copy of the second frame is in no thread.
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("edit.copy", &json!({})).unwrap();
        s.execute("edit.paste", &json!({})).unwrap();
        let copy = s.doc().unwrap().selection.objects.first().copied().unwrap();
        assert_ne!(copy, NodeId(b));
        assert_eq!(numbers(&s, copy.0).first().map(String::as_str), Some("1."));
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        let duplicate = s.execute("edit.duplicate", &json!({"dx": 10})).unwrap()["ids"][0].as_u64().unwrap();
        assert_eq!(numbers(&s, duplicate).first().map(String::as_str), Some("1."), "a duplicate");
        let layer = s.doc().unwrap().doc.layers[0].id.0;
        let moved = s.execute("layer.move", &json!({"ids": [b], "target": layer, "copy": true})).unwrap()["ids"][0].as_u64().unwrap();
        assert_eq!(numbers(&s, moved).first().map(String::as_str), Some("1."), "a copy dragged in the Layers panel");
        assert_eq!(numbers(&s, b), all(first.len() + 1), "the frame itself");
        // Convert To Point Type takes the second frame out of the thread with its text.
        s.execute("type.convertToPointType", &json!({"ids": [b]})).unwrap();
        assert!(threads(&s).is_empty());
        assert_eq!(numbers(&s, b).first().map(String::as_str), Some("1."), "the frame converted to point type");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(numbers(&s, b), all(first.len() + 1), "undo");
        // Remove Threading: each frame is a story of its own.
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("text.thread.remove", &json!({})).unwrap();
        assert!(threads(&s).is_empty());
        assert_eq!(numbers(&s, b).first().map(String::as_str), Some("1."));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(numbers(&s, b), all(first.len() + 1), "undo");
        // Release Selection of the first frame: the second one holds the whole story, from 1.
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert!(threads(&s).is_empty());
        assert_eq!(numbers(&s, b), all(1), "the frame left holds the story");
    }

    /// New Symbol from a numbered thread keeps each frame's count in the symbol's art, so the art
    /// looks as it did; Break Link puts the frames in no thread, and each counts from 1.
    #[test]
    fn a_symbol_made_of_a_numbered_thread_keeps_its_numbers() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let items = (1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
        let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": items, "area": {"width": 150, "height": 60}})).unwrap());
        let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
        let count = |t: &TextObject| t.list_count;
        let carried = text_of(&s.doc().unwrap().doc, NodeId(b)).map(count).unwrap();
        assert!(carried > 0);
        let instance = new_id(s.execute("symbol.new", &json!({"name": "Numbered"})).unwrap());
        let counts = |n: &Node| {
            let mut counts = vec![];
            n.walk(&mut |n| {
                if let NodeKind::Text(t) = &n.kind {
                    counts.push(t.list_count);
                }
            });
            counts.sort_unstable();
            counts
        };
        let doc = &s.doc().unwrap().doc;
        assert_eq!(counts(&doc.symbols.iter().find(|x| x.name == "Numbered").unwrap().art), [0, carried]);
        // Break Link puts the frames on the artboard in no thread: each counts from 1.
        s.execute("select.set", &json!({"ids": [instance]})).unwrap();
        s.execute("symbol.breakLink", &json!({})).unwrap();
        let doc = &s.doc().unwrap().doc;
        assert_eq!(doc.layers.iter().flat_map(|l| counts(l)).collect::<Vec<_>>(), [0, 0]);
    }

    /// A frame of a numbered thread pasted while an opacity mask is edited numbers from 1 in the
    /// mask art too.
    #[test]
    fn a_frame_pasted_into_an_edited_opacity_mask_numbers_from_1() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let items = (1..=10).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
        let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": items, "area": {"width": 150, "height": 60}})).unwrap());
        let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap());
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [a, b], "list": "number"})).unwrap();
        assert!(text_of(&s.doc().unwrap().doc, NodeId(b)).is_some_and(|t| t.list_count > 0));
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        s.execute("edit.copy", &json!({})).unwrap();
        let obj = new_id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 280})).unwrap());
        let m = new_id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap());
        s.execute("select.set", &json!({"ids": [obj, m]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        s.execute("transparency.editOpacityMask", &json!({})).unwrap();
        s.execute("edit.paste", &json!({})).unwrap();
        let doc = &s.doc().unwrap().doc;
        let mut counts = vec![];
        doc.node(NodeId(obj)).unwrap().extras().mask.as_ref().unwrap().art.walk(&mut |n| {
            if let NodeKind::Text(t) = &n.kind {
                counts.push(t.list_count);
            }
        });
        assert_eq!(counts, [0], "the pasted frame in the mask art");
    }

    /// A duplicate of a frame that carried a count of 99 or more numbers from 1 and has the layout
    /// bounds of what it draws: "01." leaves the text at the left indent, where a three-digit
    /// number pushed it on to the next tab stop and the paragraph took more lines.
    #[test]
    fn a_duplicated_frame_numbers_from_1_with_its_own_bounds() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 300})).unwrap();
        let story = format!("{}Itemized lists paragraph text", "x\n".repeat(110));
        let a = new_id(s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 1500}})).unwrap());
        let b = new_id(s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 100, "height": 300})).unwrap());
        s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("text.setFormat", &json!({"ids": [a, b], "number": "twoDigits"})).unwrap();
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        let copy = s.execute("edit.duplicate", &json!({"dx": 200})).unwrap()["ids"][0].as_u64().unwrap();
        let laid = |s: &Session, id: u64| {
            let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(id)) else { panic!("area type") };
            let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            (lay.markers[0].text.clone(), lay.lines.len(), lay.bounds, t.cached_bounds.unwrap())
        };
        let (number, lines, _, _) = laid(&s, b);
        let (copy_number, copy_lines, bounds, cached) = laid(&s, copy);
        assert!(number.len() == 4 && number.starts_with('1'), "a three-digit number: {number}");
        assert_eq!(copy_number, "01.");
        assert!(copy_lines < lines, "{copy_lines} {lines}");
        assert!((cached.y1 - bounds.y1).abs() < 1e-6, "the copy's bounds: {cached:?} {bounds:?}");
    }

    /// Release Selection that leaves one frame of a thread gives that frame the whole story and
    /// the layout bounds of its new text.
    #[test]
    fn the_frame_a_release_leaves_alone_has_the_bounds_of_the_whole_story() {
        let (mut s, a, b) = setup();
        s.execute("text.thread.create", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [a]})).unwrap();
        s.execute("text.thread.releaseSelection", &json!({})).unwrap();
        assert!(threads(&s).is_empty());
        assert_eq!(text(&s, b), TEXT.repeat(4));
        let Some(t) = text_of(&s.doc().unwrap().doc, NodeId(b)) else { panic!("area type") };
        let bounds = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).bounds;
        let cached = t.cached_bounds.unwrap();
        assert!((cached.y0 - bounds.y0).abs() < 1e-6 && (cached.y1 - bounds.y1).abs() < 1e-6, "{cached:?} {bounds:?}");
    }
}
