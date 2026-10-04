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
        // The story: live frames' runs, plus the text of frames deleted by this edit.
        let mut story: Vec<TextRun> = vec![];
        let mut live = vec![];
        for id in &thread {
            match (text_of(doc, *id), text_of(before, *id)) {
                (Some(t), _) => {
                    story.extend(t.runs.iter().cloned());
                    live.push(*id);
                }
                (None, Some(old)) => story.extend(old.runs.iter().cloned()),
                (None, None) => {}
            }
        }
        if !changed || live.is_empty() {
            if live.len() >= 2 {
                kept.push(live);
            }
            continue;
        }
        vectorcraft_text::edit::normalize(&mut story);
        let frames: Vec<TextObject> = live.iter().filter_map(|id| text_of(doc, *id).cloned()).collect();
        let refs: Vec<&TextObject> = frames.iter().collect();
        let parts = vectorcraft_text::thread::distribute(vectorcraft_text::FontDb::global(), &refs, &story);
        for (id, runs) in live.iter().zip(parts) {
            if let Some(NodeKind::Text(t)) = doc.node_mut(*id).map(|n| &mut n.kind)
                && t.runs != runs
            {
                t.runs = runs;
                refresh_bounds(t);
            }
        }
        if live.len() >= 2 {
            kept.push(live);
        }
    }
    doc.text_threads = kept;
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
            for id in &thread {
                if let Some(t) = text_of(d, *id) {
                    story.extend(t.runs.iter().cloned());
                }
            }
            let style = story.first().map(|r| r.style.clone()).unwrap_or_default();
            for id in &out {
                if let Some(NodeKind::Text(t)) = d.node_mut(*id).map(|n| &mut n.kind) {
                    t.runs = vec![TextRun { text: String::new(), style: style.clone() }];
                    refresh_bounds(t);
                }
            }
            if let Some(first) = keep.first()
                && let Some(NodeKind::Text(t)) = d.node_mut(*first).map(|n| &mut n.kind)
            {
                vectorcraft_text::edit::normalize(&mut story);
                t.runs = story;
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
}
