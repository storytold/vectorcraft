//! Threaded type in the script object model: a text frame's `nextFrame`, `previousFrame` and
//! `story`, `document.stories`, and each `Story` with its frames and its text, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextFrameItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Story/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Stories/> describe them.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - Assigning `nextFrame` links the last frame of a story to the first frame of another story,
//!   whose text then flows on after it; `previousFrame` links the other way. Assigning `null`
//!   breaks that link, and no text moves across it. Point type, a frame of another
//!   document, the frame itself, a frame inside a story, a link that would make a loop and a hidden
//!   or locked frame in either story are refused with nothing changed. Assigning the frame that is
//!   already linked, or `null` where there is no link, changes nothing.
//! - A `Story` is the story of the frame it was read from as that frame is threaded now; when the
//!   frame is deleted, it throws `Stale story`.
//! - `document.stories` lists one story for each type object in no thread and one for each thread,
//!   in the order of their first frames in `document.textFrames`, topmost first. `index` with a string finds the first story whose
//!   first frame has that name.
//! - The text of a story of two or more frames is its frames' text in thread order. Its ranges keep
//!   their place through edits made through them, as a frame's ranges do, and are stale after any
//!   other change to the story's text and after File › Revert. A story of one frame gives that
//!   frame's own ranges.
//! - `areaText` and `pathText` with a `baseFrame` thread the new frame after it, or before it when
//!   `postFix` is false; `postFix` is true when it is left out.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Weak};

use serde_json::json;
use vectorcraft_doc::TextKind;
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostMember, host_ref};

use super::super::{
    Answer, Doc, Host, Item, Link, NOT_FOUND, arg, boolean, class, current, doc_of, doc_ref, edit_item, find_doc, float, function, get, handle_of,
    id_of, id_parts, in_compound, item_of, item_ref, method, named, num, property, text, whole, with_doc,
};
use super::{
    ATTRS, Index, Log, Part, STALE_RANGE, frame_ids, from_script, part_spans, range_ref, read_only, text_of, this_frame, to_script, touched,
    usize_of, whole_span,
};
use crate::cmd::threads::story_of;
use crate::{DocState, Document, NodeId};

/// The error of a `Story` whose frame was deleted, or that came from an earlier run.
pub const STALE_STORY: &str = "Stale story";

/// A document's uid, the document, and the first frame of each of its stories then.
type StoryList = (u64, Weak<Document>, Rc<Vec<NodeId>>);

/// The story state of one run: the text of each story of two or more frames that a range was made
/// of, by document and the frame the `Story` was read from, and the last list of stories.
#[derive(Default)]
pub(in crate::script) struct StoryState {
    logs: RefCell<HashMap<(u64, u64), Log>>,
    list: RefCell<Option<StoryList>>,
}

/// The frames of the story `id` is a frame of, in thread order, each once, or `id` alone.
fn frames_of(st: &DocState, id: NodeId) -> Vec<NodeId> {
    let mut frames = story_of(&st.doc, id);
    let mut seen = HashSet::new();
    frames.retain(|f| seen.insert(*f));
    if frames.len() < 2 { vec![id] } else { frames }
}

/// The frames of the story `id` is a frame of, charged to the run.
fn charged_frames(h: &Host, st: &DocState, id: NodeId) -> std::result::Result<Vec<NodeId>, HostError> {
    h.charge_call(st.doc.text_threads.len() as u64)?;
    let frames = frames_of(st, id);
    h.charge_call(frames.len() as u64)?;
    Ok(frames)
}

/// Whether type object `id` is area type or type on a path.
fn threadable(st: &DocState, id: NodeId) -> bool {
    text_of(st, id).is_some_and(|t| !matches!(t.kind, TextKind::Point))
}

// ---- nextFrame and previousFrame ----

fn neighbor(h: &Host, a: &[Data], next: bool) -> Answer {
    let item = this_frame(h, a)?;
    with_doc(h, item.doc, |_, st| {
        let frames = charged_frames(h, st, item.id)?;
        let i = frames.iter().position(|f| *f == item.id);
        let other = i.and_then(|i| if next { frames.get(i + 1) } else { i.checked_sub(1).and_then(|i| frames.get(i)) });
        Ok(other.map_or(Data::Null, |f| item_ref(h, st, *f)))
    })
}

fn next_frame(h: &Host, a: &[Data]) -> Answer {
    neighbor(h, a, true)
}

fn previous_frame(h: &Host, a: &[Data]) -> Answer {
    neighbor(h, a, false)
}

/// Assigns `nextFrame` (`next`) or `previousFrame` with `text.thread.setNeighbor`.
fn set_neighbor(h: &Host, a: &[Data], next: bool) -> Answer {
    let name = if next { "nextFrame" } else { "previousFrame" };
    let item = this_frame(h, a)?;
    let other = match arg(a, 1) {
        Data::Null => None,
        d if handle_of(d, "TextFrame").is_some() => Some(item_of(h, d)?),
        _ => return Err(HostError::Type(format!("{name} must be a TextFrame or null"))),
    };
    if other.is_some_and(|o| o.doc.uid != item.doc.uid) {
        return Err(HostError::Error(format!("{name}: the two frames are in different documents")));
    }
    let now = with_doc(h, item.doc, |_, st| {
        if !threadable(st, item.id) || other.is_some_and(|o| !threadable(st, o.id)) {
            return Err(HostError::Error(format!("{name}: only area type and type on a path can be threaded")));
        }
        let frames = charged_frames(h, st, item.id)?;
        let i = frames.iter().position(|f| *f == item.id);
        let current = i.and_then(|i| if next { frames.get(i + 1) } else { i.checked_sub(1).and_then(|i| frames.get(i)) });
        Ok(current.copied())
    })?;
    if now == other.map(|o| o.id) {
        return Ok(Data::Undefined);
    }
    let side = if next { "next" } else { "previous" };
    let params = json!({"id": item.id.0, "side": side, "neighbor": other.map(|o| o.id.0)});
    edit_item(h, item, "text.thread.setNeighbor", &params).map_err(|e| match e {
        HostError::Error(m) => HostError::Error(format!("{name}: {m}")),
        e => e,
    })?;
    Ok(Data::Undefined)
}

fn set_next_frame(h: &Host, a: &[Data]) -> Answer {
    set_neighbor(h, a, true)
}

fn set_previous_frame(h: &Host, a: &[Data]) -> Answer {
    set_neighbor(h, a, false)
}

/// `TextFrame.story`.
fn frame_story(h: &Host, a: &[Data]) -> Answer {
    let item = this_frame(h, a)?;
    Ok(story_ref(h, item))
}

/// `TextRange.story`: the story of the range's frame.
fn range_story(h: &Host, a: &[Data]) -> Answer {
    let r = super::this_range(h, a)?;
    super::resolve(h, r)?;
    Ok(story_ref(h, r.item))
}

// ---- creating a frame threaded to another ----

/// The `baseFrame` and `postFix` arguments of `areaText` and `pathText`, checked before the frame
/// is made: the frame to thread the new one to, and whether the new one goes after it.
pub(super) fn base_frame(h: &Host, doc: Doc, base: &Data, post: &Data) -> std::result::Result<Option<(Item, bool)>, HostError> {
    if matches!(base, Data::Undefined | Data::Null) {
        return Ok(None);
    }
    if handle_of(base, "TextFrame").is_none() {
        return Err(HostError::Type("baseFrame must be a TextFrame".into()));
    }
    let item = item_of(h, base)?;
    if item.doc.uid != doc.uid {
        return Err(HostError::Error("baseFrame: the frame is in another document".into()));
    }
    let postfix = match post {
        Data::Undefined => true,
        d => boolean(d, "postFix")?,
    };
    with_doc(h, item.doc, |_, st| {
        if !threadable(st, item.id) {
            return Err(HostError::Error("baseFrame: only area type and type on a path can be threaded".into()));
        }
        let editable = st.doc.editable();
        if charged_frames(h, st, item.id)?.iter().any(|f| !editable(*f)) {
            return Err(HostError::Error("baseFrame: a frame of its story is hidden or locked".into()));
        }
        Ok(())
    })?;
    Ok(Some((item, postfix)))
}

/// Refuses a `baseFrame` for a path inside a compound path before the path is turned into type:
/// `text.thread.add` does not thread type there.
pub(super) fn check_base_path(h: &Host, path: Item) -> std::result::Result<(), HostError> {
    if in_compound(h, path) {
        return Err(HostError::Error("baseFrame: type made from a path inside a compound path cannot be threaded".into()));
    }
    Ok(())
}

/// Threads the new frame `id` after `base`, or before it when `postfix` is false.
pub(super) fn thread_new(h: &Host, base: Item, id: NodeId, postfix: bool) -> std::result::Result<(), HostError> {
    edit_item(h, base, "text.thread.add", &json!({"from": base.id.0, "to": id.0, "before": !postfix}))?;
    Ok(())
}

// ---- Story ----

fn story_ref(h: &Host, item: Item) -> Data {
    let [high, low] = id_parts(item.id);
    host_ref("Story", &[float(h.run), float(item.doc.uid), high, low])
}

/// The frame the story `d` was read from, which must still be a type object.
fn story_of_data(h: &Host, d: &Data) -> std::result::Result<Item, HostError> {
    let Some(handle) = handle_of(d, "Story") else { return Err(HostError::Type("this is not a Story".into())) };
    let [run, uid, high, low] = handle.as_slice() else { return Err(HostError::Type("this is not a Story".into())) };
    let doc = find_doc(h, *run, *uid).ok_or_else(|| HostError::Error(STALE_STORY.into()))?;
    let id = id_of(*high, *low).ok_or_else(|| HostError::Error(STALE_STORY.into()))?;
    match with_doc(h, doc, |_, st| text_of(st, id).is_some()) {
        true => Ok(Item { doc, id }),
        false => Err(HostError::Error(STALE_STORY.into())),
    }
}

fn this_story(h: &Host, a: &[Data]) -> std::result::Result<Item, HostError> {
    story_of_data(h, arg(a, 0))
}

fn story_frames(h: &Host, a: &[Data]) -> Answer {
    let story = this_story(h, a)?;
    with_doc(h, story.doc, |_, st| Ok(Data::Array(charged_frames(h, st, story.id)?.iter().map(|f| item_ref(h, st, *f)).collect())))
}

fn story_parent(h: &Host, a: &[Data]) -> Answer {
    let story = this_story(h, a)?;
    Ok(doc_ref(h, story.doc.uid))
}

/// The story's one frame, or `None` for a story of two or more frames.
fn single(h: &Host, story: Item) -> std::result::Result<Option<Item>, HostError> {
    with_doc(h, story.doc, |_, st| {
        let frames = charged_frames(h, st, story.id)?;
        Ok(match frames[..] {
            [one] => Some(Item { doc: story.doc, id: one }),
            _ => None,
        })
    })
}

fn story_length(h: &Host, a: &[Data]) -> Answer {
    let story = this_story(h, a)?;
    let (index, _) = with_doc(h, story.doc, |_, st| observe(h, st, story.id))?;
    Ok(num(float(index.len() as u64)))
}

/// A range or a collection of the whole story, as `frame_class` for a story of one frame and
/// `story_class` for a longer one.
fn story_whole(h: &Host, a: &[Data], frame_class: &str, story_class: &str) -> Answer {
    let story = this_story(h, a)?;
    if let Some(frame) = single(h, story)? {
        let (_, r) = whole_span(h, frame)?;
        return Ok(range_ref(h, frame, r.start, r.end, r.stamp, frame_class));
    }
    let (index, stamp) = with_doc(h, story.doc, |_, st| observe(h, st, story.id))?;
    Ok(span_ref(h, story, 0, index.len(), stamp, story_class))
}

fn story_text_range(h: &Host, a: &[Data]) -> Answer {
    story_whole(h, a, "TextRange", "StoryRange")
}

fn story_characters(h: &Host, a: &[Data]) -> Answer {
    story_whole(h, a, "Characters", "StoryCharacters")
}

fn story_words(h: &Host, a: &[Data]) -> Answer {
    story_whole(h, a, "Words", "StoryWords")
}

fn story_paragraphs(h: &Host, a: &[Data]) -> Answer {
    story_whole(h, a, "Paragraphs", "StoryParagraphs")
}

// ---- the text of a story of two or more frames ----

/// The frames of the story and the text of each, in thread order.
fn story_text(st: &DocState, frames: &[NodeId]) -> String {
    frames.iter().filter_map(|f| text_of(st, *f)).flat_map(|t| t.runs.iter().map(|r| r.text.as_str())).collect()
}

/// The story's index and its generation now, read again when its document changed. The stories
/// read from the frames of one story share one index of its text while the document is unchanged.
fn observe(h: &Host, st: &DocState, anchor: NodeId) -> std::result::Result<(Rc<Index>, u64), HostError> {
    if text_of(st, anchor).is_none() {
        return Err(HostError::Error(STALE_STORY.into()));
    }
    let mut logs = h.dom.text.stories.logs.borrow_mut();
    let key = (st.uid, anchor.0);
    let now = Arc::as_ptr(&st.doc);
    let current = |l: &Log| std::ptr::eq(l.doc.as_ptr(), now) && l.reverts == st.reverts;
    if !logs.get(&key).is_some_and(current) {
        let frames = charged_frames(h, st, anchor)?;
        // Another frame of the same story read at this state of the document has the same text;
        // a frame listed in two threads can belong to another story, so its frames must match.
        let mut shared = None;
        if frames.len() > 1 {
            h.charge_call(frames.len() as u64)?;
            if let Some((f, l)) =
                frames.iter().filter(|f| **f != anchor).find_map(|f| logs.get(&(st.uid, f.0)).filter(|l| current(l)).map(|l| (*f, l)))
                && charged_frames(h, st, f)? == frames
            {
                shared = Some(l.index.clone());
            }
        }
        let index = match shared {
            Some(index) => index,
            None => {
                let text = story_text(st, &frames);
                match logs.get(&key) {
                    Some(l) if l.reverts == st.reverts && l.index.text == text => l.index.clone(),
                    _ => Rc::new(Index::new(text)),
                }
            }
        };
        match logs.get_mut(&key) {
            Some(l) => {
                if l.reverts != st.reverts || index.text != l.index.text {
                    l.stamp += 1;
                    l.floor = l.stamp;
                    l.edits.clear();
                }
                l.index = index;
                l.doc = Arc::downgrade(&st.doc);
                l.reverts = st.reverts;
            }
            None => {
                logs.insert(key, Log { doc: Arc::downgrade(&st.doc), reverts: st.reverts, index, stamp: 0, floor: 0, edits: VecDeque::new() });
            }
        }
    }
    let l = logs.get(&key).ok_or_else(|| HostError::Error(STALE_STORY.into()))?;
    Ok((l.index.clone(), l.stamp))
}

/// A range of a story: the frame the story was read from, its span in UTF-16 units and the
/// generation it was made at.
#[derive(Clone, Copy)]
struct StorySpan {
    story: Item,
    start: usize,
    end: usize,
    stamp: u64,
}

const STORY_PARTS: [(&str, Part); 3] = [("StoryCharacters", Part::Characters), ("StoryWords", Part::Words), ("StoryParagraphs", Part::Paragraphs)];

fn span_ref(h: &Host, story: Item, s: usize, e: usize, stamp: u64, class: &str) -> Data {
    let [high, low] = id_parts(story.id);
    host_ref(class, &[float(h.run), float(story.doc.uid), high, low, float(s as u64), float(e as u64), float(stamp)])
}

/// The story span `d` names, when it is an object of one of `classes`.
fn span_of(h: &Host, d: &Data, classes: &[&str]) -> std::result::Result<StorySpan, HostError> {
    let handle = classes.iter().find_map(|c| handle_of(d, c));
    let Some(&[run, uid, high, low, s, e, stamp]) = handle.as_deref() else {
        return Err(HostError::Type(format!("this is not a {}", classes.first().copied().unwrap_or("TextRange"))));
    };
    let story = story_of_data(h, &host_ref("Story", &[run, uid, high, low])).map_err(|_| HostError::Error(STALE_RANGE.into()))?;
    let (Some(start), Some(end), Some(stamp)) = (usize_of(s), usize_of(e), whole(stamp)) else { return Err(HostError::Error(STALE_RANGE.into())) };
    Ok(StorySpan { story, start, end, stamp })
}

fn this_span(h: &Host, a: &[Data]) -> std::result::Result<StorySpan, HostError> {
    span_of(h, arg(a, 0), &["StoryRange"])
}

/// The span and index of `r` now.
fn resolve(h: &Host, r: StorySpan) -> std::result::Result<(Rc<Index>, usize, usize, u64), HostError> {
    with_doc(h, r.story.doc, |_, st| {
        let (index, stamp) = observe(h, st, r.story.id)?;
        let logs = h.dom.text.stories.logs.borrow();
        let l = logs.get(&(st.uid, r.story.id.0)).ok_or_else(|| HostError::Error(STALE_RANGE.into()))?;
        if r.stamp < l.floor || r.stamp > l.stamp {
            return Err(HostError::Error(STALE_RANGE.into()));
        }
        let (mut s, mut e) = (r.start, r.end);
        let from = l.edits.partition_point(|x| x.stamp <= r.stamp);
        for x in l.edits.range(from..) {
            let moved = |v: usize| v - x.removed + x.inserted;
            s = if s <= x.at {
                s
            } else if s >= x.at + x.removed {
                moved(s)
            } else {
                x.at
            };
            // An end inside the removed text goes to the end of the new text.
            e = if e < x.at || (e == x.at && x.removed > 0) {
                e
            } else if e >= x.at + x.removed {
                moved(e)
            } else {
                x.at + x.inserted
            };
        }
        let e = e.min(index.len());
        Ok((index, s.min(e), e, stamp))
    })
}

/// Records an edit the model made to the story: `removed` units at `at` replaced by `inserted`.
/// When the text is not what that edit gives, every earlier range goes stale. The ranges of the
/// same story read from its other frames record the edit too, when they knew the text before it.
fn record(h: &Host, story: Item, before: &Index, at: usize, removed: usize, inserted: &str) -> std::result::Result<(), HostError> {
    with_doc(h, story.doc, |_, st| {
        if text_of(st, story.id).is_none() {
            return Err(HostError::Error(STALE_STORY.into()));
        }
        let frames = charged_frames(h, st, story.id)?;
        let now = story_text(st, &frames);
        let (a, b) = (before.byte(at, false), before.byte(at + removed, true));
        let expected = before.text.len() - (b - a) + inserted.len() == now.len()
            && now.as_bytes().starts_with(&before.text.as_bytes()[..a])
            && now.as_bytes().ends_with(&before.text.as_bytes()[b..])
            && now.get(a..a + inserted.len()) == Some(inserted);
        let members: HashSet<u64> = frames.iter().map(|f| f.0).collect();
        let index = Rc::new(Index::new(now));
        let inserted = inserted.encode_utf16().count();
        let mut logs = h.dom.text.stories.logs.borrow_mut();
        let mut found = false;
        for (&(uid, anchor), l) in logs.iter_mut() {
            if uid != st.uid || !members.contains(&anchor) {
                continue;
            }
            let own = anchor == story.id.0;
            let current = l.reverts == st.reverts;
            if !own {
                h.charge_call(l.index.text.len() as u64)?;
                if !expected || !current || l.index.text != before.text {
                    continue;
                }
            }
            found |= own;
            if expected && current {
                l.push(at, removed, inserted);
            } else {
                l.stamp += 1;
                l.floor = l.stamp;
                l.edits.clear();
            }
            l.index = index.clone();
            l.doc = Arc::downgrade(&st.doc);
            l.reverts = st.reverts;
        }
        if found { Ok(()) } else { Err(HostError::Error(STALE_RANGE.into())) }
    })
}

/// Replaces bytes `a..b` of the story's text with `insert`, a frame at a time from the end, with
/// `text.editRange`: the frames before the one an edit changes keep their text, so `a` stays in
/// place while the frames after it flow again.
fn replace(h: &Host, story: Item, a: usize, mut b: usize, insert: &str) -> std::result::Result<(), HostError> {
    loop {
        let spans: Vec<(NodeId, usize, usize)> = with_doc(h, story.doc, |_, st| {
            if text_of(st, story.id).is_none() {
                return Err(HostError::Error(STALE_STORY.into()));
            }
            let mut at = 0;
            let spans = charged_frames(h, st, story.id)?
                .into_iter()
                .map(|f| {
                    let n: usize = text_of(st, f).map_or(0, |t| t.runs.iter().map(|r| r.text.len()).sum());
                    at += n;
                    (f, at - n, at)
                })
                .collect();
            Ok(spans)
        })?;
        let target =
            if b > a { spans.iter().find(|(_, x, y)| *x < b && b <= *y) } else { spans.iter().find(|(_, x, y)| *x < a && a <= *y).or(spans.first()) };
        let Some(&(f, x, _)) = target else { return Err(HostError::Error(STALE_RANGE.into())) };
        let item = Item { doc: story.doc, id: f };
        if x <= a {
            edit_item(h, item, "text.editRange", &json!({"id": f.0, "start": a - x, "end": b - x, "insert": insert}))?;
            return Ok(());
        }
        edit_item(h, item, "text.editRange", &json!({"id": f.0, "start": 0, "end": b - x, "insert": ""}))?;
        b = x;
    }
}

fn write_contents(h: &Host, r: StorySpan, value: &Data) -> Answer {
    let Data::String(v) = value else { return Err(HostError::Type("contents must be a string".into())) };
    let insert = from_script(v);
    let (index, s, e, _) = resolve(h, r)?;
    let (a, b) = (index.byte(s, false), index.byte(e, true));
    if index.text[a..b] == insert {
        return Ok(Data::Undefined);
    }
    let (s, e) = (index.text[..a].encode_utf16().count(), index.text[..b].encode_utf16().count());
    replace(h, r.story, a, b, &insert)?;
    record(h, r.story, &index, s, e - s, &insert)?;
    Ok(Data::Undefined)
}

fn span_contents(h: &Host, a: &[Data]) -> Answer {
    let (index, s, e, _) = resolve(h, this_span(h, a)?)?;
    let (x, y) = (index.byte(s, false), index.byte(e, true));
    Ok(text(to_script(&index.text[x..y])))
}

fn span_set_contents(h: &Host, a: &[Data]) -> Answer {
    write_contents(h, this_span(h, a)?, arg(a, 1))
}

fn span_remove(h: &Host, a: &[Data]) -> Answer {
    write_contents(h, this_span(h, a)?, &text(""))
}

fn span_start(h: &Host, a: &[Data]) -> Answer {
    let (_, s, _, _) = resolve(h, this_span(h, a)?)?;
    Ok(num(float(s as u64)))
}

fn span_end(h: &Host, a: &[Data]) -> Answer {
    let (_, _, e, _) = resolve(h, this_span(h, a)?)?;
    Ok(num(float(e as u64)))
}

fn span_length(h: &Host, a: &[Data]) -> Answer {
    let (_, s, e, _) = resolve(h, this_span(h, a)?)?;
    Ok(num(float((e - s) as u64)))
}

fn span_story(h: &Host, a: &[Data]) -> Answer {
    let r = this_span(h, a)?;
    resolve(h, r)?;
    Ok(story_ref(h, r.story))
}

fn span_paragraph_length(h: &Host, a: &[Data]) -> Answer {
    let (index, s, e, _) = resolve(h, this_span(h, a)?)?;
    let p = touched(&index.paras, s, e).next().and_then(|i| index.paras.get(i)).map_or(0, |(x, y)| y - x);
    Ok(num(float(p as u64)))
}

fn span_part(h: &Host, a: &[Data], class: &str) -> Answer {
    let r = this_span(h, a)?;
    let (_, s, e, stamp) = resolve(h, r)?;
    Ok(span_ref(h, r.story, s, e, stamp, class))
}

fn span_characters(h: &Host, a: &[Data]) -> Answer {
    span_part(h, a, "StoryCharacters")
}

fn span_words(h: &Host, a: &[Data]) -> Answer {
    span_part(h, a, "StoryWords")
}

fn span_paragraphs(h: &Host, a: &[Data]) -> Answer {
    span_part(h, a, "StoryParagraphs")
}

fn this_part(h: &Host, a: &[Data]) -> std::result::Result<(StorySpan, Part), HostError> {
    let part = STORY_PARTS.iter().find(|(c, _)| handle_of(arg(a, 0), c).is_some()).map(|(_, p)| *p);
    let part = part.ok_or_else(|| HostError::Type("this is not a collection of text".into()))?;
    Ok((span_of(h, arg(a, 0), &STORY_PARTS.map(|(c, _)| c))?, part))
}

fn part_typename(h: &Host, a: &[Data]) -> Answer {
    let (r, part) = this_part(h, a)?;
    resolve(h, r)?;
    Ok(text(match part {
        Part::Characters => "Characters",
        Part::Words => "Words",
        Part::Paragraphs => "Paragraphs",
    }))
}

fn span_typename(h: &Host, a: &[Data]) -> Answer {
    resolve(h, this_span(h, a)?)?;
    Ok(text("TextRange"))
}

fn story_typename(h: &Host, a: &[Data]) -> Answer {
    this_story(h, a)?;
    Ok(text("Story"))
}

fn part_length(h: &Host, a: &[Data]) -> Answer {
    let (r, part) = this_part(h, a)?;
    let (index, s, e, _) = resolve(h, r)?;
    Ok(num(float(part_spans(&index, part, s, e).0.len() as u64)))
}

fn part_parent(h: &Host, a: &[Data]) -> Answer {
    let (r, _) = this_part(h, a)?;
    let (_, s, e, stamp) = resolve(h, r)?;
    Ok(span_ref(h, r.story, s, e, stamp, "StoryRange"))
}

fn part_at(h: &Host, a: &[Data]) -> Answer {
    let (r, part) = this_part(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = usize_of(*n) else { return Ok(Data::Undefined) };
    let (index, s, e, stamp) = resolve(h, r)?;
    let (items, spans) = part_spans(&index, part, s, e);
    let Some(k) = items.start.checked_add(i).filter(|k| *k < items.end) else { return Ok(Data::Undefined) };
    let span = match spans {
        None => index.char_span(k),
        Some(list) => list.get(k).copied(),
    };
    Ok(span.map_or(Data::Undefined, |(x, y)| span_ref(h, r.story, x, y, stamp, "StoryRange")))
}

// ---- Stories ----

fn doc_stories_impl(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("Stories", &[float(h.run), float(doc.uid)]))
}

/// `document.stories`.
pub(in crate::script::dom) fn doc_stories(link: &Link) -> HostMember {
    get(link, doc_stories_impl)
}

/// The first frame of each story of the document in paint order, kept while the document is
/// unchanged.
fn heads(h: &Host, st: &DocState) -> std::result::Result<Rc<Vec<NodeId>>, HostError> {
    if let Some((uid, doc, ids)) = &*h.dom.text.stories.list.borrow()
        && *uid == st.uid
        && std::ptr::eq(doc.as_ptr(), Arc::as_ptr(&st.doc))
    {
        return Ok(ids.clone());
    }
    let ids = frame_ids(h, st);
    h.charge_call(ids.len() as u64 + st.doc.text_threads.iter().map(|t| t.len() as u64).sum::<u64>())?;
    // A frame listed by an earlier thread belongs to that one, as `story_of` finds it.
    let (mut threaded, mut first) = (HashSet::new(), HashSet::new());
    let mut seen = HashSet::new();
    for thread in &st.doc.text_threads {
        let live: Vec<NodeId> = thread.iter().copied().filter(|f| threadable(st, *f) && seen.insert(*f)).collect();
        if live.len() >= 2 {
            first.insert(live[0]);
            threaded.extend(live);
        }
    }
    let heads: Vec<NodeId> = ids.iter().copied().filter(|id| !threaded.contains(id) || first.contains(id)).collect();
    let heads = Rc::new(heads);
    *h.dom.text.stories.list.borrow_mut() = Some((st.uid, Arc::downgrade(&st.doc), heads.clone()));
    Ok(heads)
}

fn stories_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "Stories")
}

fn stories_length(h: &Host, a: &[Data]) -> Answer {
    let doc = stories_doc(h, a)?;
    let n = with_doc(h, doc, |_, st| heads(h, st).map(|l| l.len()))?;
    Ok(num(float(n as u64)))
}

fn stories_typename(h: &Host, a: &[Data]) -> Answer {
    stories_doc(h, a)?;
    Ok(text("Stories"))
}

fn stories_parent(h: &Host, a: &[Data]) -> Answer {
    let doc = stories_doc(h, a)?;
    Ok(doc_ref(h, doc.uid))
}

fn stories_at(h: &Host, a: &[Data]) -> Answer {
    let doc = stories_doc(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = usize_of(*n) else { return Ok(Data::Undefined) };
    let id = with_doc(h, doc, |_, st| heads(h, st).map(|l| l.get(i).copied()))?;
    Ok(id.map_or(Data::Undefined, |id| story_ref(h, Item { doc, id })))
}

/// `stories.index(key)`: by number, or by the name of a story's first frame.
fn stories_index(h: &Host, a: &[Data]) -> Answer {
    let doc = stories_doc(h, a)?;
    match arg(a, 1) {
        Data::Number(_) => match stories_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        Data::String(name) => {
            let id = with_doc(h, doc, |_, st| {
                let list = heads(h, st)?;
                h.charge_call(list.len() as u64)?;
                Ok::<_, HostError>(list.iter().copied().find(|i| st.doc.node(*i).and_then(|n| n.name.as_deref()) == Some(name.as_str())))
            })?;
            id.map(|id| story_ref(h, Item { doc, id })).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
        }
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

// ---- classes ----

/// A member of the range of a story of two or more frames that is not available yet: reading or
/// assigning it throws, after the check that the range is still there.
fn range_later(link: &Link, name: &'static str) -> (String, HostMember) {
    let message = format!("TextRange.{name} is not available yet");
    let l = link.clone();
    let f: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16)?;
        resolve(&h, this_span(&h, &a)?)?;
        Err(HostError::Error(message.clone()))
    });
    (name.to_string(), HostMember::Property { get: f.clone(), set: f })
}

/// A member of `Story` that is not available yet: reading or assigning it throws, after the check
/// that the story is still there.
fn story_later(link: &Link, name: &'static str) -> (String, HostMember) {
    let message = format!("Story.{name} is not available yet");
    let l = link.clone();
    let f: HostFunction = Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16)?;
        this_story(&h, &a)?;
        Err(HostError::Error(message.clone()))
    });
    (name.to_string(), HostMember::Property { get: f.clone(), set: f })
}

/// The members of `TextFrame` this module adds.
pub(super) fn frame_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![
        ("nextFrame", property(l, next_frame, set_next_frame)),
        ("previousFrame", property(l, previous_frame, set_previous_frame)),
        ("story", get(l, frame_story)),
    ])
}

/// `TextRange.story`.
pub(super) fn range_member(l: &Link) -> (String, HostMember) {
    ("story".into(), get(l, range_story))
}

/// `Story`, `Stories` and the range and collections of the text of a story of two or more frames.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let mut story = named(vec![
        ("typename", get(l, story_typename)),
        ("parent", get(l, story_parent)),
        ("length", get(l, story_length)),
        ("textFrames", get(l, story_frames)),
        ("textRange", get(l, story_text_range)),
        ("characters", get(l, story_characters)),
        ("words", get(l, story_words)),
        ("paragraphs", get(l, story_paragraphs)),
    ]);
    story.extend(["lines", "insertionPoints", "textRanges", "textSelection"].into_iter().map(|m| story_later(l, m)));
    let mut range = named(vec![
        ("typename", get(l, span_typename)),
        ("parent", get(l, span_story)),
        ("story", get(l, span_story)),
        ("contents", property(l, span_contents, span_set_contents)),
        ("start", read_only(l, span_start, "TextRange", "start")),
        ("end", read_only(l, span_end, "TextRange", "end")),
        ("characterOffset", read_only(l, span_start, "TextRange", "characterOffset")),
        ("length", read_only(l, span_length, "TextRange", "length")),
        ("characters", get(l, span_characters)),
        ("words", get(l, span_words)),
        ("paragraphs", get(l, span_paragraphs)),
        ("getParagraphLength", method(l, span_paragraph_length)),
        ("remove", method(l, span_remove)),
    ]);
    range.extend(
        [
            "lines",
            "insertionPoints",
            "textRanges",
            "characterAttributes",
            "paragraphAttributes",
            "kerning",
            "characterStyles",
            "paragraphStyles",
            "textSelection",
            "changeCaseTo",
            "select",
            "deSelect",
            "duplicate",
            "move",
            "getTextRunLength",
            "getLocalCharOverridesJSON",
            "getLocalParaOverridesJSON",
        ]
        .into_iter()
        .chain(ATTRS.iter().map(|(n, _)| *n))
        .map(|m| range_later(l, m)),
    );
    let part = |name: &'static str| {
        class(
            name,
            None,
            named(vec![("typename", get(l, part_typename)), ("length", get(l, part_length)), ("parent", get(l, part_parent))]),
            Some(function(l, part_at)),
        )
    };
    vec![
        class("Story", None, story, None),
        class("StoryRange", None, range, None),
        part("StoryCharacters"),
        part("StoryWords"),
        part("StoryParagraphs"),
        class(
            "Stories",
            None,
            named(vec![
                ("typename", get(l, stories_typename)),
                ("length", get(l, stories_length)),
                ("parent", get(l, stories_parent)),
                ("index", method(l, stories_index)),
            ]),
            Some(function(l, stories_at)),
        ),
    ]
}
