//! Type in the script object model: `document.textFrames`, each `TextFrame`, the `TextRange` of its
//! text with the `characters`, `words` and `paragraphs` in it, their `characterAttributes` and
//! `paragraphAttributes`, and `app.textFonts`, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextFrameItems/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextFrameItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextRange/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/CharacterAttributes/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextFonts/> describe them.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - Offsets and lengths count UTF-16 code units, as a JavaScript string does, so
//!   `frame.contents.length` is `frame.textRange.length`. An offset inside a surrogate pair moves
//!   out to the whole character. One item of `characters` is one code point.
//! - `contents` gives each paragraph break as `\r` and each forced line break as `\u0003`; written
//!   text may use `\r`, `\n` or `\r\n` for a paragraph break.
//! - A word is a run of characters that are not white space. A paragraph ends after its break, and
//!   a range's `words` and `paragraphs` are the whole words and paragraphs it touches.
//! - A range keeps its place while the script edits the frame through the object model: an edit
//!   before it moves it, an edit over one of its ends takes that end to the edge of the edit's new
//!   text, so a range the edit covers holds the new text, and an insertion at one of its ends joins
//!   it. When a member finds the frame's text different from the text the model last saw, because
//!   `vc.run` or an undo changed it, every range made before is stale and throws
//!   `Stale text range`; a change reversed before that read leaves the ranges as they were. After
//!   File › Revert every range made before is stale, even when the file read back holds the same
//!   text.
//! - The model keeps the frame's last [`MAX_EDITS`] edits; a range made before an edit that is no
//!   longer kept is stale.
//! - Setting a range's `length` moves its end and leaves the text as it is; each read of a member
//!   that gives a range gives a new object.
//! - `add` on `characters`, `words` or `paragraphs` inserts after the collection's last item, and
//!   `addBefore` before its first. An added word is kept apart from the characters next to it by a
//!   space, and an added paragraph by a paragraph break.
//! - A character attribute read on a range whose characters differ gives the first character's
//!   value; set on an empty range it changes nothing. A paragraph attribute reads the first
//!   paragraph the range touches and sets every one of them. A text range has the members of
//!   both its `characterAttributes` and its `paragraphAttributes`.
//! - `fillColor` and `strokeColor` take and give color objects as a path's do; a `NoColor` takes
//!   the paint away. `strokeWeight` is 0 to 1000 points.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Weak};

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{CharStyle, Justify, TextKind, TextObject};
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostItem, HostMember, Realm, RunError, host_ref};

use super::{
    Answer, Doc, Host, Item, Link, NOT_FOUND, STALE_ITEM, all_editable, arg, boolean, class, colors, command, doc_of, doc_ref, edit_item, editable,
    float, frame, function, get, handle_of, id_parts, item_of, item_ref, method, named, num, number, on_doc, page_items, point, property, text,
    typename_of, whole, with_doc,
};
use crate::{DocState, Document, NodeId, NodeKind};

/// The error of a text range, or of a collection of one, whose frame's text changed other than
/// through the object model.
pub const STALE_RANGE: &str = "Stale text range";
/// The most edits a frame keeps for moving its ranges; a range made before the oldest kept edit
/// is stale.
const MAX_EDITS: usize = 100_000;

/// A frame's text with the offsets of its characters, words and paragraphs, in UTF-16 units.
pub(super) struct Index {
    text: String,
    /// The UTF-16 offset of each character's start, and the text's length last.
    chars: Vec<usize>,
    /// The byte offset of each character's start, and the text's length in bytes last.
    bytes: Vec<usize>,
    words: Vec<(usize, usize)>,
    paras: Vec<(usize, usize)>,
}

impl Index {
    fn new(text: String) -> Self {
        let mut chars = Vec::with_capacity(text.len() + 1);
        let mut bytes = Vec::with_capacity(text.len() + 1);
        let (mut words, mut paras) = (Vec::new(), Vec::new());
        let (mut u, mut word, mut para) = (0, None, 0);
        for (b, c) in text.char_indices() {
            chars.push(u);
            bytes.push(b);
            if c.is_whitespace() {
                if let Some(w) = word.take() {
                    words.push((w, u));
                }
            } else if word.is_none() {
                word = Some(u);
            }
            u += c.len_utf16();
            if c == '\n' {
                paras.push((para, u));
                para = u;
            }
        }
        if let Some(w) = word {
            words.push((w, u));
        }
        paras.push((para, u));
        chars.push(u);
        bytes.push(text.len());
        Self { text, chars, bytes, words, paras }
    }

    fn len(&self) -> usize {
        self.chars.last().copied().unwrap_or(0)
    }

    /// The byte offset of UTF-16 offset `u`; inside a surrogate pair, the start of its character,
    /// or with `up` the end of it.
    fn byte(&self, u: usize, up: bool) -> usize {
        let i = self.chars.partition_point(|&c| c < u);
        match self.chars.get(i) {
            Some(&c) if c == u || up => self.bytes.get(i).copied().unwrap_or(self.text.len()),
            _ => i.checked_sub(1).and_then(|i| self.bytes.get(i)).copied().unwrap_or(0),
        }
    }

    /// The characters whose start is in `s..e`, each as a UTF-16 span.
    fn chars_in(&self, s: usize, e: usize) -> std::ops::Range<usize> {
        let count = self.chars.len().saturating_sub(1);
        let a = self.chars[..count].partition_point(|&c| c < s);
        let b = self.chars[..count].partition_point(|&c| c < e);
        a..b.max(a)
    }

    fn char_span(&self, i: usize) -> Option<(usize, usize)> {
        Some((*self.chars.get(i)?, *self.chars.get(i + 1)?))
    }
}

/// The words or paragraphs of `spans` that `s..e` touches: those it overlaps, or, for an empty
/// range, the one with a character at it, or the last one when it ends there. Only an empty range
/// touches the empty paragraph after a break at the end of the text.
fn touched(spans: &[(usize, usize)], s: usize, e: usize) -> std::ops::Range<usize> {
    if s == e {
        let i = spans.partition_point(|&(_, y)| y <= s);
        if spans.get(i).is_some_and(|&(x, _)| x <= s) {
            return i..i + 1;
        }
        return match spans.last() {
            Some(&(_, y)) if i == spans.len() && y == s => i - 1..i,
            _ => i..i,
        };
    }
    let a = spans.partition_point(|&(_, y)| y <= s);
    let b = spans.partition_point(|&(x, _)| x < e);
    a..b.max(a)
}

#[derive(Clone, Copy)]
struct Edit {
    stamp: u64,
    at: usize,
    removed: usize,
    inserted: usize,
}

/// What the object model knows of one frame's text: the document it was read from and its revert
/// count then, the text, the edits made through the model, and the oldest range generation still
/// valid.
struct Log {
    doc: Weak<Document>,
    reverts: u64,
    index: Rc<Index>,
    stamp: u64,
    floor: u64,
    edits: VecDeque<Edit>,
}

impl Log {
    /// Records an edit the model made: `removed` units at `at` replaced by `inserted` units. Past
    /// [`MAX_EDITS`] the oldest edit is dropped, and ranges made before it go stale.
    fn push(&mut self, at: usize, removed: usize, inserted: usize) {
        self.stamp += 1;
        self.edits.push_back(Edit { stamp: self.stamp, at, removed, inserted });
        if self.edits.len() > MAX_EDITS
            && let Some(dropped) = self.edits.pop_front()
        {
            self.floor = dropped.stamp;
        }
    }
}

/// A document's uid, the document, and the ids of its type objects then.
type FrameList = (u64, Weak<Document>, Rc<Vec<NodeId>>);

/// Word or paragraph spans, in UTF-16 units.
type Spans = [(usize, usize)];

/// The text state of one run.
#[derive(Default)]
pub(in crate::script) struct TextState {
    frames: RefCell<HashMap<(u64, u64), Log>>,
    /// A document's uid, the document, and the ids of its type objects, topmost first.
    list: RefCell<Option<FrameList>>,
    /// The fonts `app.textFonts` lists, as family and style, read once a run; a font a character
    /// names that is not installed is added at the end.
    fonts: RefCell<Option<Vec<(String, String)>>>,
    /// The number of text range objects made, which numbers the next one.
    made: Cell<u64>,
    /// The span of each range object, by its number, whose `length` the script set.
    resized: RefCell<HashMap<u64, (usize, usize, u64)>>,
}

fn text_of(st: &DocState, id: NodeId) -> Option<&TextObject> {
    match &st.doc.node(id)?.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    }
}

fn same_text(t: &TextObject, s: &str) -> bool {
    let mut rest = s;
    for r in &t.runs {
        match rest.strip_prefix(r.text.as_str()) {
            Some(after) => rest = after,
            None => return false,
        }
    }
    rest.is_empty()
}

/// The frame's index and its generation now, read again when its document changed. After File ›
/// Revert every earlier range goes stale, even when the file read back holds the same text.
fn observe(h: &Host, st: &DocState, id: NodeId) -> std::result::Result<(Rc<Index>, u64), HostError> {
    let t = text_of(st, id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let mut frames = h.dom.text.frames.borrow_mut();
    let key = (st.uid, id.0);
    let now = Arc::as_ptr(&st.doc);
    match frames.get_mut(&key) {
        Some(l) if std::ptr::eq(l.doc.as_ptr(), now) && l.reverts == st.reverts => {}
        Some(l) => {
            if l.reverts != st.reverts || !same_text(t, &l.index.text) {
                l.stamp += 1;
                l.floor = l.stamp;
                l.edits.clear();
                l.index = Rc::new(Index::new(t.plain_text()));
            }
            l.doc = Arc::downgrade(&st.doc);
            l.reverts = st.reverts;
        }
        None => {
            let l = Log {
                doc: Arc::downgrade(&st.doc),
                reverts: st.reverts,
                index: Rc::new(Index::new(t.plain_text())),
                stamp: 0,
                floor: 0,
                edits: VecDeque::new(),
            };
            frames.insert(key, l);
        }
    }
    let l = frames.get(&key).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    Ok((l.index.clone(), l.stamp))
}

/// A text range: its frame, its span in UTF-16 units and the generation it was made at.
#[derive(Clone, Copy)]
struct Span {
    item: Item,
    start: usize,
    end: usize,
    stamp: u64,
}

/// The span and index of `r` now.
fn resolve(h: &Host, r: Span) -> std::result::Result<(Rc<Index>, usize, usize, u64), HostError> {
    with_doc(h, r.item.doc, |_, st| {
        let (index, stamp) = observe(h, st, r.item.id)?;
        let frames = h.dom.text.frames.borrow();
        let l = frames.get(&(st.uid, r.item.id.0)).ok_or_else(|| HostError::Error(STALE_RANGE.into()))?;
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
        let len = index.len();
        let e = e.min(len);
        Ok((index, s.min(e), e, stamp))
    })
}

/// Runs command `id` with `params` on the text frame of `item`. Throws `Cannot edit`, with nothing
/// changed, when the frame or a layer or group it is in is hidden or locked.
fn edit_text(h: &Host, item: Item, id: &str, params: &Value) -> std::result::Result<Data, HostError> {
    if !with_doc(h, item.doc, |_, st| editable(&st.doc, item.id)) {
        return Err(HostError::Error(format!("Cannot edit TextFrame {}: it, or a layer or group it is in, is hidden or locked", item.id.0)));
    }
    edit_item(h, item, id, params)
}

/// Records an edit the model made to the frame: `removed` units at `at` replaced by `inserted`.
/// When the text is not what that edit gives, every earlier range goes stale.
fn record(h: &Host, item: Item, before: &Index, at: usize, removed: usize, inserted: &str) -> std::result::Result<(), HostError> {
    with_doc(h, item.doc, |_, st| {
        let t = text_of(st, item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        let now = t.plain_text();
        let (a, b) = (before.byte(at, false), before.byte(at + removed, true));
        let expected = before.text.len() - (b - a) + inserted.len() == now.len()
            && now.as_bytes().starts_with(&before.text.as_bytes()[..a])
            && now.as_bytes().ends_with(&before.text.as_bytes()[b..])
            && now.get(a..a + inserted.len()) == Some(inserted);
        let mut frames = h.dom.text.frames.borrow_mut();
        let l = frames.get_mut(&(st.uid, item.id.0)).ok_or_else(|| HostError::Error(STALE_RANGE.into()))?;
        if expected && l.reverts == st.reverts {
            l.push(at, removed, inserted.encode_utf16().count());
        } else {
            l.stamp += 1;
            l.floor = l.stamp;
            l.edits.clear();
        }
        l.index = Rc::new(Index::new(now));
        l.doc = Arc::downgrade(&st.doc);
        l.reverts = st.reverts;
        Ok(())
    })
}

// ---- handles ----

fn usize_of(n: f64) -> Option<usize> {
    whole(n).and_then(|v| usize::try_from(v).ok())
}

/// A new object of `class` for the span `s..e` of the frame `item` at generation `stamp`. Each
/// object is numbered, so setting one range's `length` leaves every other range as it is.
fn range_ref(h: &Host, item: Item, s: usize, e: usize, stamp: u64, class: &str) -> Data {
    let [high, low] = id_parts(item.id);
    let made = &h.dom.text.made;
    made.set(made.get() + 1);
    host_ref(class, &[float(h.run), float(item.doc.uid), high, low, float(s as u64), float(e as u64), float(stamp), float(made.get())])
}

/// The span `d` names, when it is an object of one of `classes`.
fn span_of(h: &Host, d: &Data, classes: &[&str]) -> std::result::Result<Span, HostError> {
    span_and_number(h, d, classes).map(|(span, _)| span)
}

/// The span `d` names and the number of its object, when it is an object of one of `classes`.
fn span_and_number(h: &Host, d: &Data, classes: &[&str]) -> std::result::Result<(Span, u64), HostError> {
    let handle = classes.iter().find_map(|c| handle_of(d, c));
    let Some(&[run, uid, high, low, s, e, stamp, number]) = handle.as_deref() else {
        return Err(HostError::Type(format!("this is not a {}", classes.first().copied().unwrap_or("TextRange"))));
    };
    let frame = item_of(h, &host_ref("TextFrame", &[run, uid, high, low])).map_err(|_| HostError::Error(STALE_RANGE.into()))?;
    let (Some(start), Some(end), Some(stamp), Some(number)) = (usize_of(s), usize_of(e), whole(stamp), whole(number)) else {
        return Err(HostError::Error(STALE_RANGE.into()));
    };
    let (start, end, stamp) = h.dom.text.resized.borrow().get(&number).copied().unwrap_or((start, end, stamp));
    Ok((Span { item: frame, start, end, stamp }, number))
}

/// The text frame `this` names.
fn this_frame(h: &Host, a: &[Data]) -> std::result::Result<Item, HostError> {
    if handle_of(arg(a, 0), "TextFrame").is_none() {
        return Err(HostError::Type("this is not a TextFrame".into()));
    }
    item_of(h, arg(a, 0))
}

/// The whole text of the frame as a span.
fn whole_span(h: &Host, item: Item) -> std::result::Result<(Rc<Index>, Span), HostError> {
    with_doc(h, item.doc, |_, st| {
        let (index, stamp) = observe(h, st, item.id)?;
        let len = index.len();
        Ok((index, Span { item, start: 0, end: len, stamp }))
    })
}

fn this_range(h: &Host, a: &[Data]) -> std::result::Result<Span, HostError> {
    span_of(h, arg(a, 0), &["TextRange"])
}

/// Fails when `d` is not a text range, or is a stale one.
pub(super) fn check_range(h: &Host, d: &Data) -> std::result::Result<(), HostError> {
    resolve(h, span_of(h, d, &["TextRange"])?).map(|_| ())
}

/// The span of `this`: a text range, or a text frame's whole text.
fn this_text(h: &Host, a: &[Data], class: &str) -> std::result::Result<Span, HostError> {
    if handle_of(arg(a, 0), "TextFrame").is_some() {
        return Ok(whole_span(h, item_of(h, arg(a, 0))?)?.1);
    }
    span_of(h, arg(a, 0), &[class])
}

// ---- contents ----

/// The model's text as `contents` gives it.
fn to_script(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\n' => '\r',
            '\u{2028}' => '\u{3}',
            c => c,
        })
        .collect()
}

/// Text a script wrote, as the model keeps it.
fn from_script(s: &str) -> String {
    s.replace("\r\n", "\n")
        .chars()
        .map(|c| match c {
            '\r' => '\n',
            '\u{3}' => '\u{2028}',
            c => c,
        })
        .collect()
}

fn read_contents(h: &Host, r: Span) -> Answer {
    let (index, s, e, _) = resolve(h, r)?;
    let (a, b) = (index.byte(s, false), index.byte(e, true));
    Ok(text(to_script(&index.text[a..b])))
}

/// The text a script gives as `contents`: a string, or a number as `String(number)` writes it.
fn contents_of(value: &Data) -> std::result::Result<String, HostError> {
    match value {
        Data::String(v) => Ok(from_script(v)),
        Data::Number(n) => Ok(vectorcraft_js::number_to_string(*n)),
        _ => Err(HostError::Type("contents must be a string or a number".into())),
    }
}

/// Replaces the text of `r` with `value`, as one `text.editRange`.
fn write_contents(h: &Host, r: Span, value: &Data) -> Answer {
    let insert = contents_of(value)?;
    let (index, s, e, _) = resolve(h, r)?;
    let (a, b) = (index.byte(s, false), index.byte(e, true));
    if index.text[a..b] == insert {
        return Ok(Data::Undefined);
    }
    let (s, e) = (index.text[..a].encode_utf16().count(), index.text[..b].encode_utf16().count());
    edit_text(h, r.item, "text.editRange", &json!({"id": r.item.id.0, "start": a, "end": b, "insert": insert}))?;
    record(h, r.item, &index, s, e - s, &insert)?;
    Ok(Data::Undefined)
}

fn frame_contents(h: &Host, a: &[Data]) -> Answer {
    let (_, r) = whole_span(h, this_frame(h, a)?)?;
    read_contents(h, r)
}

fn frame_set_contents(h: &Host, a: &[Data]) -> Answer {
    let (_, r) = whole_span(h, this_frame(h, a)?)?;
    write_contents(h, r, arg(a, 1))
}

fn range_contents(h: &Host, a: &[Data]) -> Answer {
    read_contents(h, this_range(h, a)?)
}

fn range_set_contents(h: &Host, a: &[Data]) -> Answer {
    write_contents(h, this_range(h, a)?, arg(a, 1))
}

// ---- TextFrame ----

fn frame_kind(h: &Host, a: &[Data]) -> Answer {
    let item = this_frame(h, a)?;
    with_doc(h, item.doc, |_, st| {
        let t = text_of(st, item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        Ok(text(match t.kind {
            TextKind::Point => "TextType.POINTTEXT",
            TextKind::Area { .. } => "TextType.AREATEXT",
            TextKind::OnPath { .. } => "TextType.PATHTEXT",
        }))
    })
}

fn frame_orientation(h: &Host, a: &[Data]) -> Answer {
    let item = this_frame(h, a)?;
    with_doc(h, item.doc, |_, st| {
        let t = text_of(st, item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        Ok(text(if t.vertical { "TextOrientation.VERTICAL" } else { "TextOrientation.HORIZONTAL" }))
    })
}

fn frame_range(h: &Host, a: &[Data]) -> Answer {
    let item = this_frame(h, a)?;
    let (_, r) = whole_span(h, item)?;
    Ok(range_ref(h, item, r.start, r.end, r.stamp, "TextRange"))
}

fn sub(h: &Host, a: &[Data], class: &str) -> Answer {
    let r = this_text(h, a, "TextRange")?;
    let (_, s, e, stamp) = resolve(h, r)?;
    Ok(range_ref(h, r.item, s, e, stamp, class))
}

fn characters(h: &Host, a: &[Data]) -> Answer {
    sub(h, a, "Characters")
}

fn words(h: &Host, a: &[Data]) -> Answer {
    sub(h, a, "Words")
}

fn paragraphs(h: &Host, a: &[Data]) -> Answer {
    sub(h, a, "Paragraphs")
}

fn character_attributes(h: &Host, a: &[Data]) -> Answer {
    sub(h, a, "CharacterAttributes")
}

fn paragraph_attributes(h: &Host, a: &[Data]) -> Answer {
    sub(h, a, "ParagraphAttributes")
}

/// The id a command's result, as the command gave it, holds under `ids` (the first) or `id`. It is
/// read from the JSON, where an id above 2^53 is still exact.
fn result_id(v: &Value) -> Option<NodeId> {
    v.get("ids").and_then(|ids| ids.get(0)).or_else(|| v.get("id")).and_then(Value::as_u64).map(NodeId)
}

fn new_ref(h: &Host, doc: Doc, id: Option<NodeId>) -> Answer {
    let id = id.ok_or_else(|| HostError::Error("the command made no object".into()))?;
    let s = h.session.borrow();
    let st = s.documents().iter().find(|d| d.uid == doc.uid).ok_or_else(|| HostError::Error(super::STALE_DOCUMENT.into()))?;
    Ok(item_ref(h, st, id))
}

/// `createOutline()`: Type › Create Outlines for the frame alone. → the group of outlines, or
/// `null` when the type has nothing to outline and the command removed it.
fn frame_outline(h: &Host, a: &[Data]) -> Answer {
    let item = this_frame(h, a)?;
    let (_, r) = on_doc(h, item.doc.uid, true, |h| {
        command(h, "select.set", &json!({"ids": [item.id.0]}))?;
        super::command_value(h, "type.createOutlines", &json!({}))
    })?;
    match result_id(&r) {
        Some(id) => new_ref(h, item.doc, Some(id)),
        None => Ok(Data::Null),
    }
}

/// Converts area type to point type (`area` false) or point type to area type.
fn convert(h: &Host, a: &[Data], to_area: bool) -> Answer {
    let item = this_frame(h, a)?;
    let kind_ok = with_doc(h, item.doc, |_, st| {
        text_of(st, item.id).map(|t| if to_area { matches!(t.kind, TextKind::Point) } else { matches!(t.kind, TextKind::Area { .. }) })
    });
    match kind_ok {
        None => return Err(HostError::Error(STALE_ITEM.into())),
        Some(false) => return Err(HostError::Error(format!("the text frame is not {} type", if to_area { "point" } else { "area" }))),
        Some(true) => {}
    }
    let id = if to_area { "type.convertToAreaType" } else { "type.convertToPointType" };
    let (_, r) = on_doc(h, item.doc.uid, true, |h| super::command_value(h, id, &json!({"ids": [item.id.0]})))?;
    new_ref(h, item.doc, result_id(&r).or(Some(item.id)))
}

fn frame_to_point(h: &Host, a: &[Data]) -> Answer {
    convert(h, a, false)
}

fn frame_to_area(h: &Host, a: &[Data]) -> Answer {
    convert(h, a, true)
}

// ---- TextRange ----

fn range_parent(h: &Host, a: &[Data]) -> Answer {
    let r = this_range(h, a)?;
    resolve(h, r)?;
    Ok(with_doc(h, r.item.doc, |_, st| item_ref(h, st, r.item.id)))
}

fn range_start(h: &Host, a: &[Data]) -> Answer {
    let (_, s, _, _) = resolve(h, this_range(h, a)?)?;
    Ok(num(float(s as u64)))
}

fn range_end(h: &Host, a: &[Data]) -> Answer {
    let (_, _, e, _) = resolve(h, this_range(h, a)?)?;
    Ok(num(float(e as u64)))
}

fn range_length(h: &Host, a: &[Data]) -> Answer {
    let (_, s, e, _) = resolve(h, this_range(h, a)?)?;
    Ok(num(float((e - s) as u64)))
}

/// Setting `length` moves the range's end to that many UTF-16 units after its start; the text
/// does not change. A length that is not a whole number from 0 to the units left in the frame
/// throws.
fn range_set_length(h: &Host, a: &[Data]) -> Answer {
    let (r, number) = span_and_number(h, arg(a, 0), &["TextRange"])?;
    let (index, s, _, stamp) = resolve(h, r)?;
    let n = super::number(arg(a, 1), "length")?;
    let left = index.len() - s;
    let Some(n) = usize_of(n).filter(|n| *n <= left) else {
        return Err(HostError::Range(format!("length must be a whole number from 0 to {left}")));
    };
    h.dom.text.resized.borrow_mut().insert(number, (s, s + n, stamp));
    Ok(Data::Undefined)
}

/// `remove()`: deletes the range's text.
fn range_remove(h: &Host, a: &[Data]) -> Answer {
    write_contents(h, this_range(h, a)?, &text(""))
}

/// `getParagraphLength()`: the length of the first paragraph the range touches, its break included.
fn range_paragraph_length(h: &Host, a: &[Data]) -> Answer {
    let (index, s, e, _) = resolve(h, this_range(h, a)?)?;
    let p = touched(&index.paras, s, e).next().and_then(|i| index.paras.get(i)).map_or(0, |(x, y)| y - x);
    Ok(num(float(p as u64)))
}

/// `getTextRunLength()`: the length from the range's start to the end of the run of characters
/// with one style it starts in, within the range.
fn range_run_length(h: &Host, a: &[Data]) -> Answer {
    let r = this_range(h, a)?;
    let (index, s, e, _) = resolve(h, r)?;
    let b = index.byte(s, false);
    let run_end = with_doc(h, r.item.doc, |_, st| {
        let t = text_of(st, r.item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        let mut at = 0;
        for run in &t.runs {
            at += run.text.len();
            if at > b {
                return Ok(at);
            }
        }
        Ok(at)
    })?;
    let u = index.text[..run_end.min(index.text.len())].encode_utf16().count();
    Ok(num(float((u.min(e).max(s) - s) as u64)))
}

// ---- collections of a range ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Characters,
    Words,
    Paragraphs,
}

const PARTS: [(&str, Part); 3] = [("Characters", Part::Characters), ("Words", Part::Words), ("Paragraphs", Part::Paragraphs)];

fn part_of(d: &Data) -> Option<Part> {
    PARTS.iter().find(|(c, _)| handle_of(d, c).is_some()).map(|(_, p)| *p)
}

fn this_part(h: &Host, a: &[Data]) -> std::result::Result<(Span, Part), HostError> {
    let part = part_of(arg(a, 0)).ok_or_else(|| HostError::Type("this is not a collection of text".into()))?;
    Ok((span_of(h, arg(a, 0), &["Characters", "Words", "Paragraphs"])?, part))
}

/// The spans of a collection, as a range of item numbers and the spans to look them up in.
fn part_spans(index: &Index, part: Part, s: usize, e: usize) -> (std::ops::Range<usize>, Option<&Spans>) {
    match part {
        Part::Characters => (index.chars_in(s, e), None),
        Part::Words => (touched(&index.words, s, e), Some(&index.words)),
        Part::Paragraphs => (touched(&index.paras, s, e), Some(&index.paras)),
    }
}

fn part_length(h: &Host, a: &[Data]) -> Answer {
    let (r, part) = this_part(h, a)?;
    let (index, s, e, _) = resolve(h, r)?;
    Ok(num(float(part_spans(&index, part, s, e).0.len() as u64)))
}

fn part_parent(h: &Host, a: &[Data]) -> Answer {
    let (r, _) = this_part(h, a)?;
    let (_, s, e, stamp) = resolve(h, r)?;
    Ok(range_ref(h, r.item, s, e, stamp, "TextRange"))
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
    Ok(span.map_or(Data::Undefined, |(x, y)| range_ref(h, r.item, x, y, stamp, "TextRange")))
}

/// Where `add` puts its text: before or after the relative object.
fn text_placement(d: &Data) -> std::result::Result<bool, HostError> {
    match d {
        Data::Undefined => Ok(true),
        Data::String(s) if s == "ElementPlacement.PLACEBEFORE" || s == "ElementPlacement.PLACEATBEGINNING" => Ok(false),
        Data::String(s) if s == "ElementPlacement.PLACEAFTER" || s == "ElementPlacement.PLACEATEND" => Ok(true),
        _ => Err(HostError::Type("insertionLocation must be PLACEBEFORE, PLACEAFTER, PLACEATBEGINNING or PLACEATEND".into())),
    }
}

/// `add(contents, relativeObject?, insertionLocation?)` and, with `before`, `addBefore(contents)`:
/// inserts `contents` after the collection's last item (`addBefore`: before its first item), or
/// before or after `relativeObject`, a text range or text frame of the same frame, as one
/// `text.editRange`. A word gets a space on each side that touches another character that is not
/// white space; a paragraph gets a paragraph break on each side that touches another paragraph.
/// → the range of `contents`.
fn part_add(h: &Host, a: &[Data], before: bool) -> Answer {
    let (r, part) = this_part(h, a)?;
    let contents = contents_of(arg(a, 1))?;
    let (index, s, e, _) = resolve(h, r)?;
    // The place, and whether it is the end of a range, which rounds up to a character boundary.
    let (at, after) = if before || matches!(arg(a, 2), Data::Undefined) {
        let (items, spans) = part_spans(&index, part, s, e);
        let span = |k: usize| match spans {
            None => index.char_span(k),
            Some(list) => list.get(k).copied(),
        };
        if before {
            (items.clone().next().and_then(span).map_or(s, |(x, _)| x), false)
        } else {
            (items.clone().next_back().and_then(span).map_or(e, |(_, y)| y), true)
        }
    } else {
        let rel = arg(a, 2);
        let target = if handle_of(rel, "TextFrame").is_some() {
            whole_span(h, item_of(h, rel)?)?.1
        } else {
            span_of(h, rel, &["TextRange"]).map_err(|_| HostError::Type("relativeObject must be a TextRange or a TextFrame".into()))?
        };
        if target.item.doc.uid != r.item.doc.uid || target.item.id != r.item.id {
            return Err(HostError::Error("relativeObject must be text of the same text frame".into()));
        }
        let (_, ts, te, _) = resolve(h, target)?;
        if text_placement(arg(a, 3))? { (te, true) } else { (ts, false) }
    };
    // A place inside a character (between the two halves of a surrogate pair) moves to its edge.
    let b = index.byte(at, after);
    let at = index.text[..b].encode_utf16().count();
    if contents.is_empty() {
        let (_, stamp) = with_doc(h, r.item.doc, |_, st| observe(h, st, r.item.id))?;
        return Ok(range_ref(h, r.item, at, at, stamp, "TextRange"));
    }
    let (prev, next) = (index.text[..b].chars().next_back(), index.text[b..].chars().next());
    let (lead, trail) = match part {
        Part::Characters => ("", ""),
        Part::Words => {
            let apart = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace());
            let lead = if apart(prev) && !contents.starts_with(char::is_whitespace) { " " } else { "" };
            let trail = if apart(next) && !contents.ends_with(char::is_whitespace) { " " } else { "" };
            (lead, trail)
        }
        Part::Paragraphs => (
            if prev.is_some_and(|c| c != '\n') && !contents.starts_with('\n') { "\n" } else { "" },
            if next.is_some() && !contents.ends_with('\n') { "\n" } else { "" },
        ),
    };
    let insert = format!("{lead}{contents}{trail}");
    edit_text(h, r.item, "text.editRange", &json!({"id": r.item.id.0, "start": b, "end": b, "insert": insert}))?;
    record(h, r.item, &index, at, 0, &insert)?;
    let (_, stamp) = with_doc(h, r.item.doc, |_, st| observe(h, st, r.item.id))?;
    let start = at + lead.encode_utf16().count();
    Ok(range_ref(h, r.item, start, start + contents.encode_utf16().count(), stamp, "TextRange"))
}

fn part_add_after(h: &Host, a: &[Data]) -> Answer {
    part_add(h, a, false)
}

fn part_add_before(h: &Host, a: &[Data]) -> Answer {
    part_add(h, a, true)
}

// ---- character attributes ----

#[derive(Clone, Copy)]
enum Attr {
    Size,
    Leading,
    AutoLeading,
    Tracking,
    BaselineShift,
    HorizontalScale,
    VerticalScale,
    Rotation,
    Underline,
    StrikeThrough,
    TextFont,
    FillColor,
    StrokeColor,
    StrokeWeight,
}

const ATTRS: [(&str, Attr); 14] = [
    ("size", Attr::Size),
    ("leading", Attr::Leading),
    ("autoLeading", Attr::AutoLeading),
    ("tracking", Attr::Tracking),
    ("baselineShift", Attr::BaselineShift),
    ("horizontalScale", Attr::HorizontalScale),
    ("verticalScale", Attr::VerticalScale),
    ("rotation", Attr::Rotation),
    ("underline", Attr::Underline),
    ("strikeThrough", Attr::StrikeThrough),
    ("textFont", Attr::TextFont),
    ("fillColor", Attr::FillColor),
    ("strokeColor", Attr::StrokeColor),
    ("strokeWeight", Attr::StrokeWeight),
];

/// The styles of the characters in bytes `a..b`, effective as the Character panel shows them, with
/// the paragraph they are in: one entry for each run and paragraph the range covers. For an empty
/// range, the style of the character before it, or of the first one. With `first`, only the first
/// entry.
fn styles_in(t: &TextObject, a: usize, b: usize, first: bool) -> std::result::Result<Vec<(CharStyle, usize)>, HostError> {
    let mut out = Vec::new();
    let (mut at, mut para) = (0, 0);
    let effective = |s: &CharStyle| t.effective_char_style(s).ok_or_else(|| HostError::Error("the text's transform cannot be measured".into()));
    for run in &t.runs {
        let end = at + run.text.len();
        let bytes = run.text.as_bytes();
        let breaks = |from: usize, to: usize| bytes.get(from - at..to - at).map_or(0, |s| s.iter().filter(|&&c| c == b'\n').count());
        if a == b && at < a.max(1) && a <= end {
            let lo = a.saturating_sub(1).max(at);
            out.push((effective(&run.style)?, para + breaks(at, lo)));
            break;
        }
        if a < end && at < b {
            let (lo, hi) = (a.max(at), b.min(end));
            let style = effective(&run.style)?;
            let mut p = para + breaks(at, lo);
            out.push((style.clone(), p));
            if first {
                return Ok(out);
            }
            for (k, _) in bytes.get(lo - at..hi - at).unwrap_or_default().iter().enumerate().filter(|&(_, &c)| c == b'\n') {
                p += 1;
                if lo + k + 1 < hi {
                    out.push((style.clone(), p));
                }
            }
        }
        para += bytes.iter().filter(|&&c| c == b'\n').count();
        at = end;
    }
    if out.is_empty()
        && let Some(r) = t.runs.first()
    {
        out.push((effective(&r.style)?, 0));
    }
    Ok(out)
}

/// The bytes in `a..b` whose characters have auto leading, each stretch with the leading its
/// characters' auto leading sets them with, in document points; touching stretches with the same
/// leading are one.
fn auto_leading_spans(t: &TextObject, a: usize, b: usize) -> std::result::Result<Vec<(usize, usize, f64)>, HostError> {
    let mut out: Vec<(usize, usize, f64)> = Vec::new();
    let mut push = |from: usize, to: usize, leading: f64| match out.last_mut() {
        Some(last) if last.1 == from && same(&num(last.2), &num(leading)) => last.1 = to,
        _ => out.push((from, to, leading)),
    };
    let (mut at, mut para) = (0, 0);
    for run in &t.runs {
        let end = at + run.text.len();
        let bytes = run.text.as_bytes();
        if a < end && at < b {
            let style = t.effective_char_style(&run.style).ok_or_else(|| HostError::Error("the text's transform cannot be measured".into()))?;
            if style.leading.is_none() {
                let (lo, hi) = (a.max(at), b.min(end));
                let auto = |p: usize| t.para_at(p).justification.auto_leading_for(style.size);
                let mut p = para + bytes.get(..lo - at).unwrap_or_default().iter().filter(|&&c| c == b'\n').count();
                let mut from = lo;
                for (k, _) in bytes.get(lo - at..hi - at).unwrap_or_default().iter().enumerate().filter(|&(_, &c)| c == b'\n') {
                    let to = lo + k + 1;
                    push(from, to, auto(p));
                    p += 1;
                    from = to;
                }
                if from < hi {
                    push(from, hi, auto(p));
                }
            }
        }
        para += bytes.iter().filter(|&&c| c == b'\n').count();
        at = end;
    }
    Ok(out)
}

fn same(a: &Data, b: &Data) -> bool {
    match (a, b) {
        (Data::Number(x), Data::Number(y)) => (x - y).abs() < 1e-9,
        _ => a == b,
    }
}

fn attr_value(h: &Host, attr: Attr, t: &TextObject, s: &CharStyle, para: usize) -> Data {
    match attr {
        Attr::Size => num(s.size),
        Attr::Leading => num(s.leading.unwrap_or_else(|| t.para_at(para).justification.auto_leading_for(s.size))),
        Attr::AutoLeading => Data::Bool(s.leading.is_none()),
        Attr::Tracking => num(s.tracking),
        Attr::BaselineShift => num(s.baseline_shift),
        Attr::HorizontalScale => num(s.h_scale),
        Attr::VerticalScale => num(s.v_scale),
        Attr::Rotation => num(s.rotation),
        Attr::Underline => Data::Bool(s.underline),
        Attr::StrikeThrough => Data::Bool(s.strikethrough),
        Attr::TextFont => font_ref(h, &s.font_family, &s.font_style),
        Attr::FillColor => colors::of_paint(&s.fill),
        Attr::StrokeColor => colors::of_paint(&s.stroke),
        Attr::StrokeWeight => num(s.stroke_width * stroke_scale(t)),
    }
}

/// Document points per text-space point of a character stroke, as `text.setRangeStyle` scales a
/// `strokeWidth` it is given.
fn stroke_scale(t: &TextObject) -> f64 {
    vectorcraft_doc::Scaling::factor(t.xf).filter(|k| k.is_finite() && *k > 1e-12).unwrap_or(1.0)
}

/// The `text.setRangeStyle` parameter for `attr` set to `v`, and the value it reads as after.
fn attr_param(h: &Host, attr: Attr, name: &str, v: &Data, first: (&TextObject, &CharStyle, usize)) -> std::result::Result<(Value, Data), HostError> {
    let finite = || number(v, name);
    Ok(match attr {
        Attr::Size => {
            let n = finite()?;
            if !(0.1..=1296.0).contains(&n) {
                return Err(HostError::Range("size must be from 0.1 to 1296 points".into()));
            }
            (json!({"size": n}), num(n))
        }
        Attr::Leading => {
            let n = finite()?;
            if !(0.1..=5000.0).contains(&n) {
                return Err(HostError::Range("leading must be from 0.1 to 5000 points".into()));
            }
            (json!({"leading": n}), num(n))
        }
        Attr::AutoLeading => {
            if boolean(v, name)? {
                (json!({"leading": "auto"}), Data::Bool(true))
            } else {
                let (t, s, para) = first;
                let n = s.leading.unwrap_or_else(|| t.para_at(para).justification.auto_leading_for(s.size));
                (json!({"leading": n}), Data::Bool(false))
            }
        }
        Attr::Tracking => {
            let n = finite()?;
            if !(-1000.0..=10_000.0).contains(&n) {
                return Err(HostError::Range("tracking must be from -1000 to 10000".into()));
            }
            (json!({"tracking": n}), num(n))
        }
        Attr::BaselineShift => {
            let n = finite()?;
            if n.abs() > 1296.0 {
                return Err(HostError::Range("baselineShift must be from -1296 to 1296 points".into()));
            }
            (json!({"baselineShift": n}), num(n))
        }
        Attr::HorizontalScale | Attr::VerticalScale => {
            let n = finite()?;
            if !(1.0..=10_000.0).contains(&n) {
                return Err(HostError::Range(format!("{name} must be from 1 to 10000 percent")));
            }
            let key = if matches!(attr, Attr::HorizontalScale) { "hScale" } else { "vScale" };
            (json!({ key: n }), num(n))
        }
        Attr::Rotation => {
            let n = finite()?;
            if n.abs() > 360.0 {
                return Err(HostError::Range("rotation must be from -360 to 360 degrees".into()));
            }
            (json!({"rotation": n}), num(n))
        }
        Attr::Underline => {
            let b = boolean(v, name)?;
            (json!({"underline": b}), Data::Bool(b))
        }
        Attr::StrikeThrough => {
            let b = boolean(v, name)?;
            (json!({"strikethrough": b}), Data::Bool(b))
        }
        Attr::TextFont => {
            let (family, style) = font_of(h, v)?;
            (json!({"font": family, "style": style}), font_ref(h, &family, &style))
        }
        Attr::FillColor | Attr::StrokeColor => {
            let color = colors::assigned(v, name)?;
            let paint = color.map_or(Paint::None, Paint::solid);
            let given = color.map_or_else(|| json!("none"), |c| colors::param(&c));
            let key = if matches!(attr, Attr::FillColor) { "fill" } else { "stroke" };
            (json!({ key: given }), colors::of_paint(&paint))
        }
        Attr::StrokeWeight => {
            let n = finite()?;
            if !(0.0..=1000.0).contains(&n) {
                return Err(HostError::Range("strokeWeight must be from 0 to 1000 points".into()));
            }
            (json!({"strokeWidth": n}), num(n))
        }
    })
}

/// The span of `this` for a character or paragraph attribute: a text range, its
/// `characterAttributes` or `paragraphAttributes`, or a text frame's whole text.
fn attr_span(h: &Host, a: &[Data], class: &str) -> std::result::Result<Span, HostError> {
    if handle_of(arg(a, 0), "TextRange").is_some() {
        return this_range(h, a);
    }
    this_text(h, a, class)
}

fn read_attr(h: &Host, a: &[Data], attr: Attr) -> Answer {
    let r = attr_span(h, a, "CharacterAttributes")?;
    let (index, s, e, _) = resolve(h, r)?;
    let (ba, bb) = (index.byte(s, false), index.byte(e, true));
    with_doc(h, r.item.doc, |_, st| {
        let t = text_of(st, r.item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        Ok(styles_in(t, ba, bb, true)?.first().map_or(Data::Undefined, |(st, p)| attr_value(h, attr, t, st, *p)))
    })
}

fn write_attr(h: &Host, a: &[Data], attr: Attr, name: &str) -> Answer {
    let r = attr_span(h, a, "CharacterAttributes")?;
    let (index, s, e, _) = resolve(h, r)?;
    let (ba, bb) = (index.byte(s, false), index.byte(e, true));
    let (param, unchanged, fixed) = with_doc(h, r.item.doc, |_, st| {
        let t = text_of(st, r.item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        let styles = styles_in(t, ba, bb, false)?;
        let first = styles.first().map(|(s, p)| (s.clone(), *p)).unwrap_or_else(|| (CharStyle::default(), 0));
        let (param, wants) = attr_param(h, attr, name, arg(a, 1), (t, &first.0, first.1))?;
        // A leading set on characters with auto leading fixes it even where auto gives that value.
        let has = |st: &CharStyle, p: usize| (!matches!(attr, Attr::Leading) || st.leading.is_some()) && same(&attr_value(h, attr, t, st, p), &wants);
        // An empty range changes nothing, also one inside a character that `ba..bb` widens to it.
        let unchanged = s == e || ba == bb || styles.iter().all(|(st, p)| has(st, *p));
        let fixed = match (attr, &wants) {
            (Attr::AutoLeading, Data::Bool(false)) if !unchanged => Some(auto_leading_spans(t, ba, bb)?),
            _ => None,
        };
        if let Some(bad) = fixed.iter().flatten().map(|(_, _, l)| *l).find(|l| !(0.1..=5000.0).contains(l)) {
            return Err(HostError::Range(format!("autoLeading cannot be turned off: the auto leading here is {bad} points, outside 0.1 to 5000")));
        }
        Ok::<_, HostError>((param, unchanged, fixed))
    })?;
    if unchanged {
        return Ok(Data::Undefined);
    }
    if let Some(spans) = fixed {
        // Each character with auto leading keeps the leading it is set with now.
        super::atomically(h, |h| {
            for (start, end, leading) in &spans {
                let p = json!({"id": r.item.id.0, "start": start, "end": end, "leading": leading});
                edit_text(h, r.item, "text.setRangeStyle", &p)?;
            }
            Ok(())
        })?;
        return Ok(Data::Undefined);
    }
    let mut p = param;
    if let Some(o) = p.as_object_mut() {
        o.insert("id".into(), json!(r.item.id.0));
        o.insert("start".into(), json!(ba));
        o.insert("end".into(), json!(bb));
    }
    let answer = edit_text(h, r.item, "text.setRangeStyle", &p)?;
    if let Attr::TextFont = attr {
        let wants = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        font_applied(h, r.item, (ba, bb), (&wants("font"), &wants("style")), &answer)?;
    }
    Ok(Data::Undefined)
}

/// Fails, after the font is set, when `text.setRangeStyle` warned that the font is not available
/// or when characters in bytes `a..b` kept another font because Missing Glyph Protection is on.
fn font_applied(h: &Host, item: Item, (a, b): (usize, usize), (family, style): (&str, &str), answer: &Data) -> std::result::Result<(), HostError> {
    if let Data::Object(o) = answer
        && let Some((_, Data::String(w))) = o.iter().find(|(k, _)| k == "warning")
    {
        return Err(HostError::Error(format!("textFont: {w}")));
    }
    let kept = with_doc(h, item.doc, |_, st| {
        let Some(t) = text_of(st, item.id) else { return 0 };
        let mut at = 0;
        let mut kept = 0;
        for run in &t.runs {
            let end = at + run.text.len();
            let other = !run.style.font_family.eq_ignore_ascii_case(family) || !run.style.font_style.eq_ignore_ascii_case(style);
            if other && a < end && at < b {
                kept += run.text.get(a.max(at) - at..b.min(end) - at).map_or(0, |s| s.chars().count());
            }
            at = end;
        }
        kept
    });
    if kept > 0 {
        let noun = if kept == 1 { "character, which keeps its" } else { "characters, which keep their" };
        return Err(HostError::Error(format!("textFont: the font has no glyph for {kept} {noun} font because Missing Glyph Protection is on")));
    }
    Ok(())
}

// ---- paragraph attributes ----

#[derive(Clone, Copy)]
enum ParaAttr {
    Justification,
    FirstLineIndent,
    LeftIndent,
    RightIndent,
    SpaceBefore,
    SpaceAfter,
}

const PARA_ATTRS: [(&str, ParaAttr); 6] = [
    ("justification", ParaAttr::Justification),
    ("firstLineIndent", ParaAttr::FirstLineIndent),
    ("leftIndent", ParaAttr::LeftIndent),
    ("rightIndent", ParaAttr::RightIndent),
    ("spaceBefore", ParaAttr::SpaceBefore),
    ("spaceAfter", ParaAttr::SpaceAfter),
];

const JUSTIFICATIONS: [(&str, &str); 7] = [
    ("LEFT", "left"),
    ("CENTER", "center"),
    ("RIGHT", "right"),
    ("FULLJUSTIFYLASTLINELEFT", "justifyLeft"),
    ("FULLJUSTIFYLASTLINECENTER", "justifyCenter"),
    ("FULLJUSTIFYLASTLINERIGHT", "justifyRight"),
    ("FULLJUSTIFY", "justifyAll"),
];

/// The text of paragraph `i` of `index`, its break included.
fn para_text(index: &Index, i: usize) -> &str {
    index.paras.get(i).and_then(|&(x, y)| index.text.get(index.byte(x, false)..index.byte(y, false))).unwrap_or_default()
}

/// The value of `attr` in paragraph `i` of `t`, whose text `index` holds. Auto alignment reads as
/// the side it aligns to: right in a right-to-left paragraph, left otherwise.
fn para_value(attr: ParaAttr, t: &TextObject, index: &Index, i: usize) -> Data {
    let p = t.para_at(i);
    match attr {
        ParaAttr::Justification => text(format!(
            "Justification.{}",
            match p.justify {
                Justify::Auto if vectorcraft_text::paragraph_is_rtl(para_text(index, i), p.direction) => "RIGHT",
                Justify::Auto | Justify::Left => "LEFT",
                Justify::Center => "CENTER",
                Justify::Right => "RIGHT",
                Justify::JustifyLeft => "FULLJUSTIFYLASTLINELEFT",
                Justify::JustifyCenter => "FULLJUSTIFYLASTLINECENTER",
                Justify::JustifyRight => "FULLJUSTIFYLASTLINERIGHT",
                Justify::JustifyAll => "FULLJUSTIFY",
            }
        )),
        ParaAttr::FirstLineIndent => num(p.first_line_indent),
        ParaAttr::LeftIndent => num(p.left_indent),
        ParaAttr::RightIndent => num(p.right_indent),
        ParaAttr::SpaceBefore => num(p.space_before),
        ParaAttr::SpaceAfter => num(p.space_after),
    }
}

/// The command and parameter that set `attr` to `v`, and the value it reads as after.
fn para_param(attr: ParaAttr, name: &str, v: &Data) -> std::result::Result<(&'static str, Value, Data), HostError> {
    if let ParaAttr::Justification = attr {
        let found = match v {
            Data::String(s) => s.strip_prefix("Justification.").and_then(|m| JUSTIFICATIONS.iter().find(|(k, _)| *k == m)),
            _ => None,
        };
        let (_, value) = found.ok_or_else(|| HostError::Type("justification must be a Justification value".into()))?;
        return Ok(("text.setStyle", json!({"justify": value}), v.clone()));
    }
    let n = number(v, name)?;
    if n.abs() > 1296.0 {
        return Err(HostError::Range(format!("{name} must be from -1296 to 1296 points")));
    }
    let key = match attr {
        ParaAttr::FirstLineIndent => "firstLineIndent",
        ParaAttr::LeftIndent => "leftIndent",
        ParaAttr::RightIndent => "rightIndent",
        ParaAttr::SpaceBefore => "spaceBefore",
        _ => "spaceAfter",
    };
    Ok(("text.setFormat", json!({ key: n }), num(n)))
}

/// The paragraphs a range touches, by number, and the bytes from the first one's start to the last
/// one's start, which select exactly those paragraphs for `text.setFormat` and `text.setStyle`.
fn para_range(index: &Index, s: usize, e: usize) -> (std::ops::Range<usize>, usize, usize) {
    let paras = touched(&index.paras, s, e);
    let start = |i: usize| index.paras.get(i).map_or(index.text.len(), |&(x, _)| index.byte(x, false));
    let (ba, bb) = (start(paras.start), start(paras.end.saturating_sub(1).max(paras.start)));
    (paras, ba, bb)
}

fn read_para(h: &Host, a: &[Data], attr: ParaAttr) -> Answer {
    let r = attr_span(h, a, "ParagraphAttributes")?;
    let (index, s, e, _) = resolve(h, r)?;
    let (paras, _, _) = para_range(&index, s, e);
    with_doc(h, r.item.doc, |_, st| {
        let t = text_of(st, r.item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        Ok(para_value(attr, t, &index, paras.start))
    })
}

fn write_para(h: &Host, a: &[Data], attr: ParaAttr, name: &str) -> Answer {
    let r = attr_span(h, a, "ParagraphAttributes")?;
    let (id, param, wants) = para_param(attr, name, arg(a, 1))?;
    let (index, s, e, _) = resolve(h, r)?;
    let (paras, ba, bb) = para_range(&index, s, e);
    let unchanged = with_doc(h, r.item.doc, |_, st| {
        let t = text_of(st, r.item.id).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
        // An explicit alignment replaces Auto even where Auto aligns to the same side.
        let auto = |i: usize| matches!(attr, ParaAttr::Justification) && t.para_at(i).justify == Justify::Auto;
        Ok::<_, HostError>(paras.clone().all(|i| !auto(i) && same(&para_value(attr, t, &index, i), &wants)))
    })?;
    if unchanged {
        return Ok(Data::Undefined);
    }
    let mut p = param;
    if let Some(o) = p.as_object_mut() {
        o.insert("id".into(), json!(r.item.id.0));
        o.insert("start".into(), json!(ba));
        o.insert("end".into(), json!(bb));
    }
    edit_text(h, r.item, id, &p)?;
    Ok(Data::Undefined)
}

// ---- TextFrames ----

fn doc_text_frames_impl(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("TextFrames", &[float(h.run), float(doc.uid)]))
}

/// `document.textFrames`.
pub(super) fn doc_text_frames(link: &Link) -> HostMember {
    get(link, doc_text_frames_impl)
}

/// The ids of the document's type objects, topmost first, kept while the document is unchanged.
fn frame_ids(h: &Host, st: &DocState) -> Rc<Vec<NodeId>> {
    if let Some((uid, doc, ids)) = &*h.dom.text.list.borrow()
        && *uid == st.uid
        && std::ptr::eq(doc.as_ptr(), Arc::as_ptr(&st.doc))
    {
        return ids.clone();
    }
    let ids: Vec<NodeId> = page_items(h, st).iter().copied().filter(|i| text_of(st, *i).is_some()).collect();
    let ids = Rc::new(ids);
    *h.dom.text.list.borrow_mut() = Some((st.uid, Arc::downgrade(&st.doc), ids.clone()));
    ids
}

fn frames_doc(h: &Host, a: &[Data]) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), "TextFrames")
}

fn frames_length(h: &Host, a: &[Data]) -> Answer {
    let doc = frames_doc(h, a)?;
    Ok(num(float(with_doc(h, doc, |_, st| frame_ids(h, st).len()) as u64)))
}

fn frames_parent(h: &Host, a: &[Data]) -> Answer {
    let doc = frames_doc(h, a)?;
    Ok(doc_ref(h, doc.uid))
}

fn frames_at(h: &Host, a: &[Data]) -> Answer {
    let doc = frames_doc(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = usize_of(*n) else { return Ok(Data::Undefined) };
    Ok(with_doc(h, doc, |_, st| frame_ids(h, st).get(i).map_or(Data::Undefined, |id| item_ref(h, st, *id))))
}

fn frames_by_name(h: &Host, a: &[Data]) -> Answer {
    let doc = frames_doc(h, a)?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    with_doc(h, doc, |_, st| {
        let ids = frame_ids(h, st);
        h.charge_call(ids.len() as u64)?;
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| item_ref(h, st, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn frames_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => frames_by_name(h, a),
        Data::Number(_) => match frames_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `TextOrientation` as `text.create`'s `vertical`, `undefined` horizontal.
fn vertical_of(d: &Data) -> std::result::Result<bool, HostError> {
    match d {
        Data::Undefined => Ok(false),
        Data::String(s) if s == "TextOrientation.HORIZONTAL" => Ok(false),
        Data::String(s) if s == "TextOrientation.VERTICAL" => Ok(true),
        _ => Err(HostError::Type("orientation must be a TextOrientation value".into())),
    }
}

/// Makes point type at `anchor` (script coordinates; `None`: the origin) with `text.create`.
fn point_text(h: &Host, doc: Doc, anchor: Option<[f64; 2]>, vertical: bool) -> Answer {
    let at = with_doc(h, doc, |_, st| Ok::<_, HostError>(frame(st)?.to_document(h.dom.coords.get(), anchor.unwrap_or([0.0, 0.0]))))?;
    let (_, r) =
        on_doc(h, doc.uid, true, |h| super::command_value(h, "text.create", &json!({"x": at.x, "y": at.y, "text": "", "vertical": vertical})))?;
    new_ref(h, doc, result_id(&r))
}

/// `add()`: empty point type at the origin.
fn frames_add(h: &Host, a: &[Data]) -> Answer {
    point_text(h, frames_doc(h, a)?, None, false)
}

/// `pointText(anchor, orientation?)`.
fn frames_point_text(h: &Host, a: &[Data]) -> Answer {
    let doc = frames_doc(h, a)?;
    let anchor = point(arg(a, 1), "anchor")?;
    point_text(h, doc, Some(anchor), vertical_of(arg(a, 2))?)
}

/// Turns the path `d` names into area type or type on a path with `text.createInPath`.
fn in_path(h: &Host, a: &[Data], on_path: bool, rest: &[usize]) -> Answer {
    let doc = frames_doc(h, a)?;
    if handle_of(arg(a, 1), "PathItem").is_none() {
        return Err(HostError::Type("textPath must be a PathItem".into()));
    }
    let path = item_of(h, arg(a, 1))?;
    if path.doc.uid != doc.uid {
        return Err(HostError::Error(super::CROSS_DOCUMENT.into()));
    }
    if rest.iter().any(|i| !matches!(arg(a, *i), Data::Undefined)) {
        return Err(HostError::Error("TextFrames: threading and path positions are not available yet".into()));
    }
    let orientation = if on_path { 4 } else { 2 };
    let vertical = vertical_of(arg(a, orientation))?;
    let mode = if on_path { "onPath" } else { "area" };
    let params = json!({"path": path.id.0, "mode": mode, "text": "", "vertical": vertical});
    let (_, r) = on_doc(h, doc.uid, true, |h| super::command_value(h, "text.createInPath", &params))?;
    new_ref(h, doc, result_id(&r))
}

/// `areaText(textPath, orientation?, baseFrame?, postFix?)`.
fn frames_area_text(h: &Host, a: &[Data]) -> Answer {
    in_path(h, a, false, &[3, 4])
}

/// `pathText(textPath, startTValue?, endTValue?, orientation?, baseFrame?, postFix?)`.
fn frames_path_text(h: &Host, a: &[Data]) -> Answer {
    in_path(h, a, true, &[2, 3, 5, 6])
}

/// `removeAll()`: deletes every type object in one command; a hidden or locked one stops it first.
fn frames_remove_all(h: &Host, a: &[Data]) -> Answer {
    let doc = frames_doc(h, a)?;
    let (ids, refused) = with_doc(h, doc, |_, st| {
        let ids = frame_ids(h, st);
        let ok = |id: &NodeId| editable(&st.doc, *id) && st.doc.node(*id).is_some_and(all_editable);
        let refused: Vec<u64> = ids.iter().filter(|i| !ok(i)).map(|i| i.0).take(20).collect();
        (ids, refused)
    });
    if !refused.is_empty() {
        let list = refused.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("cannot delete hidden or locked type: {list}")));
    }
    if !ids.is_empty() {
        on_doc(h, doc.uid, true, |h| command(h, "edit.clear", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})))?;
    }
    Ok(Data::Undefined)
}

// ---- fonts ----

/// The run's font list, read once.
fn with_fonts<T>(h: &Host, f: impl FnOnce(&mut Vec<(String, String)>) -> T) -> T {
    let mut fonts = h.dom.text.fonts.borrow_mut();
    let list = fonts.get_or_insert_with(|| {
        let db = vectorcraft_text::FontDb::global();
        let mut v = Vec::new();
        for family in db.menu_family_list().iter() {
            for style in db.styles(family) {
                v.push((family.clone(), style));
            }
        }
        v
    });
    f(list)
}

/// A `TextFont` for `family` and `style`, added to the run's list when it is not there.
fn font_ref(h: &Host, family: &str, style: &str) -> Data {
    let i = with_fonts(h, |list| match list.iter().position(|(f, s)| f.eq_ignore_ascii_case(family) && s.eq_ignore_ascii_case(style)) {
        Some(i) => i,
        None => {
            list.push((family.to_string(), style.to_string()));
            list.len() - 1
        }
    });
    host_ref("TextFont", &[float(h.run), float(i as u64)])
}

/// The family and style of the `TextFont` `d`.
fn font_of(h: &Host, d: &Data) -> std::result::Result<(String, String), HostError> {
    let Some(&[run, i]) = handle_of(d, "TextFont").as_deref() else { return Err(HostError::Type("textFont must be a TextFont".into())) };
    if whole(run) != Some(h.run) {
        return Err(HostError::Error("Stale text font".into()));
    }
    usize_of(i).and_then(|i| with_fonts(h, |list| list.get(i).cloned())).ok_or_else(|| HostError::Error("Stale text font".into()))
}

fn fonts_length(h: &Host, _: &[Data]) -> Answer {
    Ok(num(float(with_fonts(h, |l| l.len()) as u64)))
}

fn fonts_at(h: &Host, a: &[Data]) -> Answer {
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = usize_of(*n) else { return Ok(Data::Undefined) };
    let found = with_fonts(h, |l| i < l.len());
    Ok(if found { host_ref("TextFont", &[float(h.run), float(i as u64)]) } else { Data::Undefined })
}

/// `textFonts.getByName(name)`: the PostScript name of a bundled or installed face, then the family
/// and style with a space.
fn fonts_by_name(h: &Host, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    if name.len() > super::MAX_TEXT_BYTES {
        return Err(HostError::Range("the name is too long".into()));
    }
    let named = vectorcraft_text::FontDb::global().find_postscript(name).map(|f| (f.family.clone(), f.style.clone()));
    let i = with_fonts(h, |l| {
        h.charge_call(l.len() as u64)?;
        let by_ps = named.as_ref().and_then(|(f, s)| l.iter().position(|(lf, ls)| lf.eq_ignore_ascii_case(f) && ls.eq_ignore_ascii_case(s)));
        Ok::<_, HostError>(by_ps.or_else(|| l.iter().position(|(f, s)| format!("{f} {s}").eq_ignore_ascii_case(name))))
    })?;
    match (i, named) {
        (Some(i), _) => Ok(host_ref("TextFont", &[float(h.run), float(i as u64)])),
        (None, Some((family, style))) => Ok(font_ref(h, &family, &style)),
        (None, None) => Err(HostError::Error(NOT_FOUND.into())),
    }
}

fn fonts_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => fonts_by_name(h, a),
        Data::Number(_) => match fonts_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

fn font_family(h: &Host, a: &[Data]) -> Answer {
    Ok(text(font_of(h, arg(a, 0))?.0))
}

fn font_style(h: &Host, a: &[Data]) -> Answer {
    Ok(text(font_of(h, arg(a, 0))?.1))
}

/// The font's PostScript name, or its family and style joined by a hyphen when the font is not
/// installed.
fn font_name(h: &Host, a: &[Data]) -> Answer {
    let (family, style) = font_of(h, arg(a, 0))?;
    let face = vectorcraft_text::FontDb::global().face(&family, &style);
    Ok(text(face.map_or_else(|| format!("{family}-{style}").replace(' ', ""), |f| f.postscript_name())))
}

fn app_text_fonts_impl(_: &Host, _: &[Data]) -> Answer {
    Ok(host_ref("TextFonts", &[]))
}

/// `app.textFonts`.
pub(super) fn app_text_fonts(link: &Link) -> HostMember {
    get(link, app_text_fonts_impl)
}

fn app_ref(_: &Host, _: &[Data]) -> Answer {
    Ok(host_ref("Application", &[]))
}

/// A font's `parent`, `app`, once the font is checked.
fn font_parent(h: &Host, a: &[Data]) -> Answer {
    font_of(h, arg(a, 0))?;
    Ok(host_ref("Application", &[]))
}

// ---- classes ----

fn range_typename(h: &Host, a: &[Data]) -> Answer {
    resolve(h, this_range(h, a)?)?;
    Ok(text("TextRange"))
}

fn part_typename(h: &Host, a: &[Data]) -> Answer {
    let (r, part) = this_part(h, a)?;
    resolve(h, r)?;
    Ok(text(PARTS.iter().find(|(_, p)| *p == part).map_or("Characters", |(c, _)| c)))
}

fn char_attrs_typename(h: &Host, a: &[Data]) -> Answer {
    resolve(h, span_of(h, arg(a, 0), &["CharacterAttributes"])?)?;
    Ok(text("CharacterAttributes"))
}

fn para_attrs_typename(h: &Host, a: &[Data]) -> Answer {
    resolve(h, span_of(h, arg(a, 0), &["ParagraphAttributes"])?)?;
    Ok(text("ParagraphAttributes"))
}

fn frames_typename(h: &Host, a: &[Data]) -> Answer {
    frames_doc(h, a)?;
    Ok(text("TextFrames"))
}

fn font_typename(h: &Host, a: &[Data]) -> Answer {
    font_of(h, arg(a, 0))?;
    Ok(text("TextFont"))
}

/// A member listed on the help pages that a script cannot set yet. Assigning it throws, after the
/// read's own checks of `this`.
fn read_only(link: &Link, g: super::Member, class: &'static str, name: &'static str) -> HostMember {
    let message = format!("{class}.{name} cannot be set yet");
    let get = function(link, g);
    let read = get.clone();
    HostMember::Property {
        get,
        set: Rc::new(move |a: Vec<Data>| {
            read(a.into_iter().take(1).collect())?;
            Err(HostError::Error(message.clone()))
        }),
    }
}

fn attr_members(link: &Link) -> Vec<(String, HostMember)> {
    let mut out = Vec::new();
    for (name, attr) in ATTRS {
        let l = link.clone();
        let g: HostFunction = Rc::new(move |a: Vec<Data>| {
            let h = super::current(&l)?;
            h.charge_call(16 + a.iter().map(super::data_size).sum::<u64>())?;
            read_attr(&h, &a, attr)
        });
        let l = link.clone();
        let s: HostFunction = Rc::new(move |a: Vec<Data>| {
            let h = super::current(&l)?;
            h.charge_call(16 + a.iter().map(super::data_size).sum::<u64>())?;
            write_attr(&h, &a, attr, name)
        });
        out.push((name.to_string(), HostMember::Property { get: g, set: s }));
    }
    out
}

fn para_members(link: &Link) -> Vec<(String, HostMember)> {
    let mut out = Vec::new();
    for (name, attr) in PARA_ATTRS {
        let l = link.clone();
        let g: HostFunction = Rc::new(move |a: Vec<Data>| {
            let h = super::current(&l)?;
            h.charge_call(16 + a.iter().map(super::data_size).sum::<u64>())?;
            read_para(&h, &a, attr)
        });
        let l = link.clone();
        let s: HostFunction = Rc::new(move |a: Vec<Data>| {
            let h = super::current(&l)?;
            h.charge_call(16 + a.iter().map(super::data_size).sum::<u64>())?;
            write_para(&h, &a, attr, name)
        });
        out.push((name.to_string(), HostMember::Property { get: g, set: s }));
    }
    out
}

/// The enumerations type uses.
const ENUMS: &[(&str, &[&str])] = &[
    ("TextType", &["AREATEXT", "PATHTEXT", "POINTTEXT"]),
    ("TextOrientation", &["HORIZONTAL", "VERTICAL"]),
    (
        "Justification",
        &["CENTER", "FULLJUSTIFY", "FULLJUSTIFYLASTLINECENTER", "FULLJUSTIFYLASTLINELEFT", "FULLJUSTIFYLASTLINERIGHT", "LEFT", "RIGHT"],
    ),
];

/// Adds the enumerations type uses to `realm`.
pub(super) fn install_enums(realm: &mut Realm) -> std::result::Result<(), RunError> {
    for (name, members) in ENUMS {
        let items = members.iter().map(|m| (m.to_string(), HostItem::Value(text(format!("{name}.{m}"))))).collect();
        realm.define_host_object(name, items)?;
    }
    Ok(())
}

/// The classes of type, `TextFrame` among them, which inherits from `PageItem`.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let mut frame = named(vec![
        ("kind", get(l, frame_kind)),
        ("contents", property(l, frame_contents, frame_set_contents)),
        ("textRange", get(l, frame_range)),
        ("characters", get(l, characters)),
        ("words", get(l, words)),
        ("paragraphs", get(l, paragraphs)),
        ("orientation", read_only(l, frame_orientation, "TextFrame", "orientation")),
        ("createOutline", method(l, frame_outline)),
        ("convertAreaObjectToPointObject", method(l, frame_to_point)),
        ("convertPointObjectToAreaObject", method(l, frame_to_area)),
    ]);
    frame.extend(
        [
            "lines",
            "insertionPoints",
            "textRanges",
            "story",
            "nextFrame",
            "previousFrame",
            "textPath",
            "anchor",
            "antialias",
            "columnCount",
            "columnGutter",
            "rowCount",
            "rowGutter",
            "startTValue",
            "endTValue",
            "opticalAlignment",
            "matrix",
            "textSelection",
        ]
        .into_iter()
        .map(|m| super::later(l, "TextFrame", m)),
    );
    let mut range = named(vec![
        ("typename", get(l, range_typename)),
        ("parent", get(l, range_parent)),
        ("contents", property(l, range_contents, range_set_contents)),
        ("start", read_only(l, range_start, "TextRange", "start")),
        ("end", read_only(l, range_end, "TextRange", "end")),
        ("characterOffset", read_only(l, range_start, "TextRange", "characterOffset")),
        ("length", property(l, range_length, range_set_length)),
        ("characters", get(l, characters)),
        ("words", get(l, words)),
        ("paragraphs", get(l, paragraphs)),
        ("characterAttributes", get(l, character_attributes)),
        ("paragraphAttributes", get(l, paragraph_attributes)),
        ("getParagraphLength", method(l, range_paragraph_length)),
        ("getTextRunLength", method(l, range_run_length)),
        ("remove", method(l, range_remove)),
    ]);
    range.extend(attr_members(l));
    range.extend(para_members(l));
    range.extend(
        [
            "lines",
            "insertionPoints",
            "textRanges",
            "story",
            "kerning",
            "characterStyles",
            "paragraphStyles",
            "textSelection",
            "changeCaseTo",
            "select",
            "deSelect",
            "duplicate",
            "move",
            "getLocalCharOverridesJSON",
            "getLocalParaOverridesJSON",
        ]
        .into_iter()
        .map(|m| super::later(l, "TextRange", m)),
    );
    let part = |name: &'static str| {
        class(
            name,
            None,
            named(vec![
                ("typename", get(l, part_typename)),
                ("length", get(l, part_length)),
                ("parent", get(l, part_parent)),
                ("add", method(l, part_add_after)),
                ("addBefore", method(l, part_add_before)),
            ]),
            Some(function(l, part_at)),
        )
    };
    let mut chars = named(vec![("typename", get(l, char_attrs_typename))]);
    chars.extend(attr_members(l));
    let mut paras = named(vec![("typename", get(l, para_attrs_typename))]);
    paras.extend(para_members(l));
    vec![
        class("TextFrame", Some("PageItem"), frame, None),
        class("TextRange", None, range, None),
        part("Characters"),
        part("Words"),
        part("Paragraphs"),
        class("CharacterAttributes", None, chars, None),
        class("ParagraphAttributes", None, paras, None),
        class(
            "TextFrames",
            None,
            named(vec![
                ("typename", get(l, frames_typename)),
                ("length", get(l, frames_length)),
                ("parent", get(l, frames_parent)),
                ("index", method(l, frames_index)),
                ("getByName", method(l, frames_by_name)),
                ("add", method(l, frames_add)),
                ("pointText", method(l, frames_point_text)),
                ("areaText", method(l, frames_area_text)),
                ("pathText", method(l, frames_path_text)),
                ("removeAll", method(l, frames_remove_all)),
            ]),
            Some(function(l, frames_at)),
        ),
        class(
            "TextFonts",
            None,
            named(vec![
                ("typename", typename_of("TextFonts")),
                ("length", get(l, fonts_length)),
                ("parent", get(l, app_ref)),
                ("index", method(l, fonts_index)),
                ("getByName", method(l, fonts_by_name)),
            ]),
            Some(function(l, fonts_at)),
        ),
        class(
            "TextFont",
            None,
            named(vec![
                ("typename", get(l, font_typename)),
                ("name", get(l, font_name)),
                ("family", get(l, font_family)),
                ("style", get(l, font_style)),
                ("parent", get(l, font_parent)),
            ]),
            None,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::sync::Weak;

    use super::{Index, Log, MAX_EDITS, touched};

    #[test]
    fn the_edit_log_keeps_the_last_edits_and_drops_only_older_ones() {
        let mut l = Log { doc: Weak::new(), reverts: 0, index: Rc::new(Index::new(String::new())), stamp: 0, floor: 0, edits: VecDeque::new() };
        for _ in 0..MAX_EDITS {
            l.push(0, 0, 1);
        }
        assert_eq!((l.edits.len(), l.floor), (MAX_EDITS, 0));
        l.push(0, 0, 1);
        // Only the first edit is dropped: a range made after it still moves through every later edit.
        assert_eq!((l.edits.len(), l.floor), (MAX_EDITS, 1));
        assert_eq!(l.edits.front().map(|x| x.stamp), Some(2));
    }

    #[test]
    fn index_counts_utf16_units_and_splits_words_and_paragraphs() {
        let i = Index::new("a\u{1F600}b c\nd".into());
        assert_eq!(i.len(), 8);
        assert_eq!(i.chars, vec![0, 1, 3, 4, 5, 6, 7, 8]);
        assert_eq!(i.byte(2, false), 1);
        assert_eq!(i.byte(2, true), 5);
        assert_eq!(i.words, vec![(0, 4), (5, 6), (7, 8)]);
        assert_eq!(i.paras, vec![(0, 7), (7, 8)]);
        assert_eq!(i.chars_in(1, 4), 1..3);
    }

    #[test]
    fn only_an_empty_range_touches_the_empty_paragraph_at_the_end() {
        let i = Index::new("a\n".into());
        assert_eq!(i.paras, vec![(0, 2), (2, 2)]);
        assert_eq!(touched(&i.paras, 0, 2), 0..1);
        assert_eq!(touched(&i.paras, 0, 1), 0..1);
        assert_eq!(touched(&i.paras, 2, 2), 1..2);
        assert_eq!(touched(&i.paras, 1, 1), 0..1);
        let e = Index::new(String::new());
        assert_eq!(touched(&e.paras, 0, 0), 0..1);
        assert_eq!(touched(&e.words, 0, 0), 0..0);
    }
}
