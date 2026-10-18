//! Type → Threaded Text: one story flowing through area-type frames in order.
//!
//! Frames are ordinary area text objects holding their slice of the story; the document lists the
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
            "{} thread the selected area type (and closed paths, which become frames) in stacking order → {ids}",
            has_multi,
            create
        ),
        cmd!(
            "text.thread.add",
            "Thread Text",
            [],
            None,
            "{from, to?, rect?: [x, y, width, height], at?: [x, y]} thread area type `from` (a click on its out port) into one more frame, put after it in its thread: closed path or area type `to`, a new rectangle `rect`, or a new frame of `from`'s shape and size with its top-left at `at` (give one of the three) → {id}; the two frames are selected",
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

fn text_of(d: &Document, id: NodeId) -> Option<&TextObject> {
    match d.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::Area { .. }) => Some(t),
        _ => None,
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: how many threads' stories [`reflow`] copied on this thread.
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Re-flow every thread with a frame that changed between `before` and `doc` (structural sharing
/// makes "changed" a pointer comparison). Deleted frames' text flows into the remaining ones.
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
                if next.runs != t.runs || next.para != t.para || next.paras != t.paras || optical != t.optical_margin {
                    t.runs = next.runs;
                    t.para = next.para;
                    t.paras = next.paras;
                    t.optical_margin = optical;
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

/// A closed path converted to an empty area-type frame with `style`.
fn frame_from_path(n: &Node, style: vectorcraft_doc::CharStyle) -> Option<Node> {
    let NodeKind::Path { path, .. } = &n.kind else { return None };
    if !path.subpaths.first().is_some_and(|s| s.closed) {
        return None;
    }
    let mut t = TextObject::point(vectorcraft_geom::Point::ZERO, "", style);
    t.kind = TextKind::Area { frame: path.clone() };
    t.xf = vectorcraft_geom::Affine::IDENTITY;
    refresh_bounds(&mut t);
    Some(Node::new(n.id, NodeKind::Text(Box::new(t))))
}

fn create(s: &mut Session, _: &Value) -> Result<Value> {
    const C: &str = "text.thread.create";
    let ids = selected_roots(s)?;
    let d = &s.doc()?.doc;
    let style = ids.iter().find_map(|i| text_of(d, *i)).map(|t| t.first_style()).unwrap_or_default();
    let ok = ids.iter().all(|i| text_of(d, *i).is_some() || d.node(*i).is_some_and(|n| frame_from_path(n, style.clone()).is_some()));
    if ids.len() < 2 || !ok || !ids.iter().any(|i| text_of(d, *i).is_some()) {
        return Err(bad(C, "select area type and closed paths (at least one area type object)"));
    }
    let out = ids.clone();
    s.edit("Thread Text", |d, _| {
        for id in &ids {
            if text_of(d, *id).is_none() {
                let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
                let f = frame_from_path(&n, style.clone()).ok_or_else(|| bad(C, "not a closed path"))?;
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

/// `text.thread.add`: the out port's loaded text, placed.
fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "text.thread.add";
    let given = |k: &str| p.get(k).filter(|v| !v.is_null());
    let from = checked_id(s, given("from").unwrap_or(&Value::Null), C)?;
    let d = &s.doc()?.doc;
    let src = text_of(d, from).ok_or_else(|| bad(C, "`from` must be area type"))?;
    let style = src.first_style();
    // The frame to thread into: an existing object (`to`), or a new one beside `from`.
    let (to, new) = match (given("to"), given("rect"), given("at")) {
        (Some(v), None, None) => {
            let to = checked_id(s, v, C)?;
            let parent = d.parent_of(to).and_then(|i| d.node(i));
            let frame = text_of(d, to).is_some() || d.node(to).is_some_and(|n| frame_from_path(n, style.clone()).is_some());
            if to == from || !frame || parent.is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })) {
                return Err(bad(C, "`to` must be a closed path or area type other than `from`"));
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
            (NodeId(0), Some(empty_frame(src, style.clone(), frame, vectorcraft_geom::Affine::translate((x, y)))))
        }
        (None, None, Some(v)) => {
            let [x, y] = finite_numbers(v).ok_or_else(|| bad(C, "`at` must be [x, y]"))?;
            let TextKind::Area { frame } = &src.kind else { return Err(bad(C, "`from` must be area type")) };
            let b = frame.bounds().map(|b| src.xf.transform_rect_bbox(b)).ok_or_else(|| bad(C, "`from` has an empty frame"))?;
            let xf = vectorcraft_geom::Affine::translate((x - b.x0, y - b.y0)) * src.xf;
            (NodeId(0), Some(empty_frame(src, style.clone(), frame.clone(), xf)))
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
                    *d.node_mut(to).ok_or(EngineError::NoNode(to))? = frame_from_path(&n, style).ok_or_else(|| bad(C, "not a closed path"))?;
                }
                to
            }
        };
        match d.text_threads.iter_mut().find(|t| t.contains(&from)) {
            Some(t) => {
                let at = t.iter().position(|x| *x == from).map_or(t.len(), |i| i + 1);
                t.insert(at, to);
            }
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

/// An empty area-type frame `frame` placed by `xf`, with the type options of `src`.
fn empty_frame(src: &TextObject, style: vectorcraft_doc::CharStyle, frame: vectorcraft_geom::PathData, xf: vectorcraft_geom::Affine) -> TextObject {
    let mut t = src.clone();
    t.runs = vec![TextRun { text: String::new(), style, inline: None }];
    t.set_all_paras(src.para_at(0).clone());
    t.kind = TextKind::Area { frame };
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
                    refresh_bounds(t);
                }
            }
            if let Some(first) = keep.first()
                && let Some(NodeKind::Text(t)) = d.node_mut(*first).map(|n| &mut n.kind)
            {
                vectorcraft_text::edit::normalize(&mut story);
                t.runs = story;
                t.set_paragraph_styles(paras);
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
        d.text_threads.retain(|t| !t.iter().any(|i| ids.contains(i)));
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
        ] {
            assert!(s.execute("text.thread.add", &p).is_err(), "{p} was accepted");
            assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &before), "{p} changed the document");
        }
        s.execute("text.thread.add", &json!({"from": a, "to": b})).unwrap();
        let c = new_id(s.execute("text.create", &json!({"x": 10, "y": 200, "text": "x", "area": {"width": 50, "height": 20}})).unwrap());
        assert!(s.execute("text.thread.add", &json!({"from": c, "to": b})).is_err(), "a frame already threaded");
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
}
