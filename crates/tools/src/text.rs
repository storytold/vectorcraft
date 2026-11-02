//! The Type tools: Type (T), Area Type and Type on a Path.
//!
//! Type: click places point type, drag draws an area-type frame, clicking into existing text
//! places the caret. Area Type / Type on a Path: click a path to turn it into a text frame or a
//! baseline (`text.createInPath`). While type is being edited, a click that lands on no type only
//! ends the editing, and the next click makes new type. Point type placed by a click and left
//! empty is discarded when editing ends (`text.discardEmpty`); frames keep their shape. Where new
//! type goes (the click, the frame's corners) snaps to Smart Guides ([`DrawSnap`]), hovering too.
//! While no type is being edited, a press with Shift held makes the new type with the tool's
//! horizontal or vertical counterpart, and a press with Alt (Option) held makes it with Type on a
//! Path in place of Area Type or the reverse ([`TypeTool::acting`]); the pointer shows that tool.
//!
//! While editing: caret movement by character / word (Cmd or Alt) / line (Up/Down) / line ends
//! (Home/End; Cmd = whole text), Shift extends the selection, drag selects, double-click selects a
//! word and triple-click a paragraph, typing replaces the selection. Edits are coalesced into one
//! undo step per typing session: the tool keeps the styled runs locally and previews a single
//! `text.editRange` (the changed span, with its styled runs) against the session snapshot.
//! Styling a selected range goes through `text.setRangeStyle` (the Character panel reads the
//! selection from [`Tool::options`]); Alt+arrows step it by the Preferences › Type increments
//! (`type.step`). New type starts with placeholder text, selected, when Fill New Type Objects With
//! Placeholder Text is on. A space typed on its own after "1.", "1)", "A.", "A)", "*", "#", ">" or
//! "+" at the start of a paragraph in no list makes that paragraph a list (`text.startList`).
//!
//! IME: the marked text of a composition is part of the typing session (so it lays out in place),
//! underlined, until the IME commits it (it then goes through [`Tool::text_input`]) or clears it.

use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{BezPath, Point, Rect, Shape};
use vectorcraft_text::{FontDb, TextLayout, edit};

use crate::guides::DrawSnap;
use crate::{Action, Cursor, ImeSurrounding, Mods, Overlay, PointerEvent, PointerKind, SurroundingUnit, Tool, ToolContext, ToolKey};

mod plain;
mod ports;
mod tool;

#[cfg(test)]
use plain::char_range_to_bytes;
use plain::common_affixes;
pub use plain::step_key;
pub(crate) use ports::port_at;
pub use ports::{PORT_RISE_PX, PORTS_MAX, Port, PortMarks, PortState, port_centre, port_states, ported, ports_shown, push_port, thread_ports};

/// Which of the Type tools this is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Type,
    Area,
    OnPath,
}

/// A coalesced typing session: the runs at its start and the runs now.
struct Typing {
    base: Vec<TextRun>,
    cur: Vec<TextRun>,
    /// The first byte any edit of the session touched (the text before it is unchanged): the
    /// previewed span starts no later, so Return at a paragraph's end splits that paragraph (its
    /// style continues), not the next one.
    lo: usize,
}

/// IME marked text inside the typing session, not yet committed.
#[derive(Clone, Debug, PartialEq)]
struct Preedit {
    /// Byte range of the marked text in the session's runs.
    range: Range<usize>,
    /// The clause being converted, in bytes relative to `range.start` (empty: the IME's cursor).
    active: Option<Range<usize>>,
}

/// A list that typed characters started ([`TypeTool::start_typed_list`]).
struct TypedList {
    /// The type object.
    id: NodeId,
    /// The bytes of the typed characters and the space: from the paragraph's start to the caret.
    start: usize,
    end: usize,
    /// The plain text with the typed characters, before the list started.
    text: String,
    list: vectorcraft_doc::text::ParaList,
}

/// An undo step that changed the text of the type object the tool edits, in the order the history
/// holds them. The plain text before and after it is kept as a hash. For a step the tool made (a
/// typing session or a typed list), `ends` holds the byte where the changed span ends before and
/// after it, so that Undo and Redo of the step put the caret there even where the change repeats
/// the characters next to it; a step a command from outside the tool made has none.
#[derive(Clone)]
struct OwnEdit {
    id: NodeId,
    before: u64,
    after: u64,
    ends: Option<(usize, usize)>,
    /// False once Undo took the step back, until Redo makes it again.
    done: bool,
}

/// What Undo and Redo change in the tool, kept when a `command.batch` begins so that a batch that
/// fails and is rolled back puts it back ([`TypeTool::batch_notice`]).
struct HistoryState {
    editing: Option<NodeId>,
    caret: usize,
    anchor: usize,
    goal_x: Option<f64>,
    known: Option<(NodeId, String)>,
    edits: Vec<OwnEdit>,
    stepped: bool,
}

/// How many undo steps [`TypeTool::edits`] keeps.
const OWN_EDITS_MAX: usize = 256;

fn text_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The edited text's layout, keyed by the font database's generation and the text.
type CachedLayout = ((u64, TextObject), Arc<TextLayout>);

/// Process-global Type-tool edit-session counter: the IME target's replacement epoch.
///
/// A per-tool counter restarts at zero when the tool is recreated (a document switch, File ›
/// Revert), while the document keeps its uid and node ids. The same `(document, node)` would then
/// receive an epoch a still-cached native IME context already used, violating the replacement
/// contract that an old context can never edit the new owner. Sessions are therefore numbered for
/// the process lifetime; the counter only stops changing after 2^64 sessions (unreachable).
/// A mutex (not an `AtomicU64`) keeps this available on wasm targets without 64-bit atomics.
static NEXT_EDIT_SESSION: Mutex<u64> = Mutex::new(0);

/// Return the next process-wide edit-session id. It saturates at `u64::MAX` instead of wrapping to
/// zero; the final id can repeat only after 2^64 sessions.
fn next_edit_session() -> u64 {
    let mut next = NEXT_EDIT_SESSION.lock().unwrap_or_else(PoisonError::into_inner);
    let session = next.checked_add(1).unwrap_or(u64::MAX);
    *next = session;
    session
}

#[derive(Default)]
pub struct TypeTool {
    vertical: bool,
    mode: Mode,
    /// The tool to return to on Escape (e.g. when double-clicked from a selection tool).
    prev_tool: Option<String>,
    /// Text object being edited.
    editing: Option<NodeId>,
    /// Monotonic identity of this edit session (the IME target's replacement epoch): it changes
    /// whenever editing starts afresh, including on the same text object, and never on ordinary
    /// edits or caret moves. Numbered process-wide ([`NEXT_EDIT_SESSION`]), so recreating the tool
    /// (File › Revert, a document switch) cannot reuse an epoch of the same document and node.
    edit_session: u64,
    /// The point type a click just placed (edited now): discarded if empty when editing ends.
    fresh: Option<NodeId>,
    /// Caret and selection anchor (byte offsets into the plain text).
    caret: usize,
    anchor: usize,
    /// Horizontal position kept by Up/Down (text space).
    goal_x: Option<f64>,
    typing: Option<Typing>,
    preedit: Option<Preedit>,
    /// A command from outside the tool (`settle_interaction`, an external edit while typing) ended
    /// the typing session, while the text stays open for editing. A commit over that edit is
    /// application-origin unless an IME deletion has already begun a fresh typing session.
    typing_ended: bool,
    /// Where the button went down, and where new type goes from there (snapped to Smart Guides).
    press: Option<(Point, Point)>,
    /// The other corner of an area dragged out (snapped).
    drag: Option<Point>,
    /// The Type tool the press acts as, with the keys held at the press ([`Self::acting`]).
    held: Option<(Mode, bool)>,
    /// Smart Guides for where new type goes.
    snap: DrawSnap,
    /// The press landed in the edited text: dragging selects.
    selecting: bool,
    /// Click counting for double/triple click (position of the last click, count).
    clicks: (Option<Point>, u8),
    /// Styled copy of the last copied range (pasting the same plain text keeps its styles).
    clipboard: Vec<TextRun>,
    /// The list typed characters last started (`text.startList`), so that after Undo puts them back
    /// the caret goes on after them, and after Redo it goes back.
    listed: Option<TypedList>,
    /// The edited object's plain text as the tool last left or saw it, so that after Undo or Redo
    /// the caret goes to the end of the span the step changed.
    known: Option<(NodeId, String)>,
    /// The tool's own last undo steps, oldest first.
    edits: Vec<OwnEdit>,
    /// The command that ran was Undo or Redo ([`TypeTool::after_history_step`]).
    stepped: bool,
    /// The state at the start of each `command.batch` still running, innermost last.
    batches: Vec<HistoryState>,
    cache: Mutex<Option<CachedLayout>>,
}

impl TypeTool {
    pub(super) fn step_attribute(&mut self, key: ToolKey, mods: Mods, a: usize, b: usize, vertical: bool) -> Option<Vec<Action>> {
        if !mods.alt {
            return None;
        }
        let id = self.editing?;
        let (attribute, by) = plain::step_key(key, mods.shift, a == b, vertical)?;
        let mut out = self.commit();
        let by = if mods.cmd { by * 5.0 } else { by };
        out.push(Action::Exec("type.step".into(), json!({"id": id.0, "start": a, "end": b, "attribute": attribute, "by": by})));
        Some(out)
    }
    pub fn new(id: &str) -> Self {
        let mode = match id {
            "areaType" | "verticalAreaType" => Mode::Area,
            "typeOnPath" | "verticalTypeOnPath" => Mode::OnPath,
            _ => Mode::Type,
        };
        Self { mode, vertical: id.starts_with("vertical"), ..Default::default() }
    }

    /// The Type tool (which tool, and whether vertical) that a press with `mods` held acts as.
    /// While no type is being edited, Shift switches between the horizontal and the vertical tool,
    /// and Alt switches between Area Type and Type on a Path (the Type tool has no counterpart for
    /// Alt); both together do both. While type is being edited, it is the tool chosen.
    fn acting(&self, mods: Mods) -> (Mode, bool) {
        if self.editing.is_some() {
            return (self.mode, self.vertical);
        }
        let mode = match self.mode {
            Mode::Area if mods.alt => Mode::OnPath,
            Mode::OnPath if mods.alt => Mode::Area,
            mode => mode,
        };
        (mode, self.vertical != mods.shift)
    }

    fn text<'a>(cx: &'a ToolContext, id: NodeId) -> Option<&'a TextObject> {
        match &cx.doc.node(id)?.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Layout of `t`, cached by content (overlays run every frame) and by the fonts available (the
    /// web build adds fonts after startup).
    fn layout(&self, t: &TextObject) -> Arc<TextLayout> {
        let fonts = FontDb::global().generation();
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(((g, k), l)) = c.as_ref()
            && *g == fonts
            && k == t
        {
            return l.clone();
        }
        let l = Arc::new(vectorcraft_text::layout(FontDb::global(), t));
        *c = Some(((fonts, t.clone()), l.clone()));
        l
    }

    /// The edited text used to find word and paragraph boundaries for selection: the typing session's
    /// runs, or the document's current text when there is no live session or it has gone stale.
    fn plain(&self, cx: &ToolContext) -> String {
        let document = self.editing.and_then(|id| Self::text(cx, id));
        match &self.typing {
            // A session the document changed under is not the edited text any more: use the
            // document's own runs (word/paragraph selection must not index stale text).
            Some(ty) if document.is_some_and(|t| ty.cur != t.runs) => document.map(|t| t.plain_text()).unwrap_or_default(),
            Some(ty) => ty.cur.iter().map(|r| r.text.as_str()).collect(),
            None => document.map(|t| t.plain_text()).unwrap_or_default(),
        }
    }

    /// The edited plain text as borrowed run slices: the IME query copies only the bounded window
    /// it needs, never the whole document text. A typing session whose runs no longer match the
    /// document (an external edit) is not used; the document's runs are.
    fn plain_segments<'a>(&'a self, cx: &'a ToolContext) -> Option<Vec<&'a str>> {
        let id = self.editing?;
        let t = Self::text(cx, id)?;
        match &self.typing {
            Some(ty) if ty.cur == t.runs => Some(ty.cur.iter().map(|r| r.text.as_str()).collect()),
            _ => Some(t.runs.iter().map(|r| r.text.as_str()).collect()),
        }
    }

    /// The exact text [`Tool::text_input`] would insert for `s`: control characters removed, `\r`
    /// normalized to `\n`, and Typographer's Quotes applied to a straight quote. `None` when
    /// nothing would be inserted. Shared with [`Tool::text_input_is_verbatim`] so the IME receipt
    /// and the insertion cannot disagree about what was transformed.
    fn input_text(&self, cx: &ToolContext, s: &str) -> Option<String> {
        let s: String = s.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).map(|c| if c == '\r' { '\n' } else { c }).collect();
        if s.is_empty() {
            return None;
        }
        // Use Typographer's Quotes (Document Setup): a typed straight quote becomes the document's
        // opening or closing quote, by the character before it.
        let setup = &cx.doc.setup;
        if setup.typographers_quotes && matches!(s.as_str(), "\"" | "'") {
            let mut prev = self.char_before(cx, self.caret.min(self.anchor));
            return Some(setup.quotes.apply(&s, &mut prev));
        }
        Some(s)
    }

    /// The character just before byte `at` in the edited plain text, walked run by run (the quote
    /// direction needs one character, not a copy of the whole text).
    fn char_before(&self, cx: &ToolContext, at: usize) -> Option<char> {
        let segments = self.plain_segments(cx)?;
        let mut acc = 0usize;
        let mut prev = None;
        for segment in &segments {
            let end = acc + segment.len();
            if at <= acc {
                break;
            }
            let local = at.min(end).saturating_sub(acc);
            if let Some((_, c)) = segment.char_indices().take_while(|(i, _)| *i < local).last() {
                prev = Some(c);
            }
            acc = end;
        }
        prev
    }

    /// The edited object as it currently is (with the typing session's runs). A session the
    /// document changed under is dropped: the document's own runs win.
    fn current(&self, cx: &ToolContext) -> Option<TextObject> {
        let mut t = Self::text(cx, self.editing?)?.clone();
        if let Some(ty) = &self.typing
            && ty.cur == t.runs
        {
            t.runs = ty.cur.clone();
        }
        Some(t)
    }

    fn sel(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// End the typing session (one undo step). Marked text left by an IME stays as typed (as
    /// clicking away from a composition does in macOS text views); the UI interrupts the IME.
    fn commit(&mut self) -> Vec<Action> {
        self.preedit = None;
        if self.typing.take().is_some() { vec![Action::Commit] } else { vec![] }
    }

    /// Stop editing. Point type a click placed that is still empty is discarded, with the steps
    /// since its click (`text.discardEmpty`).
    fn finish(&mut self, cx: &ToolContext) -> Vec<Action> {
        let mut out = self.commit();
        if let Some(id) = self.fresh.take()
            && self.editing == Some(id)
            && Self::text(cx, id).is_some_and(|t| matches!(t.kind, TextKind::Point) && edit::runs_len(&t.runs) == 0)
        {
            out.push(Action::Exec("text.discardEmpty".into(), json!({"id": id.0})));
        }
        self.editing = None;
        self.selecting = false;
        self.goal_x = None;
        out
    }

    /// Replace the selection with `insert` (styled `runs` when given) inside the typing session.
    fn replace(&mut self, cx: &ToolContext, insert: &str, styled: Option<Vec<TextRun>>) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        let Some(t) = Self::text(cx, id) else {
            self.editing = None;
            return vec![];
        };
        let mut out = vec![];
        // The document changed under us (undo, a panel edit): start a new session.
        if self.typing.as_ref().is_some_and(|ty| ty.cur != t.runs) {
            out.push(Action::Commit);
            self.typing = None;
            self.preedit = None;
        }
        let fresh = self.typing.is_none();
        if fresh {
            out.push(Action::Begin("Typing".into()));
            self.typing_ended = false;
        }
        let ty = self.typing.get_or_insert_with(|| Typing { base: t.runs.clone(), cur: t.runs.clone(), lo: usize::MAX });
        let len = edit::runs_len(&ty.cur);
        let (a, b) = (self.caret.min(self.anchor).min(len), self.caret.max(self.anchor).min(len));
        ty.lo = ty.lo.min(a);
        let caret = match &styled {
            Some(r) => edit::replace_range_styled(&mut ty.cur, a, b, r),
            None => edit::replace_range(&mut ty.cur, a, b, insert),
        };
        self.caret = caret;
        self.anchor = caret;
        self.goal_x = None;
        // One preview for the whole session: the changed span of base → cur, with its runs.
        let base: String = ty.base.iter().map(|r| r.text.as_str()).collect();
        let cur: String = ty.cur.iter().map(|r| r.text.as_str()).collect();
        let (p, s) = common_affixes(&base, &cur, ty.lo);
        let runs = edit::slice_runs(&ty.cur, p, cur.len() - s);
        let runs = if runs.is_empty() { json!([]) } else { serde_json::to_value(&runs).unwrap_or(json!([])) };
        out.push(Action::Preview("text.editRange".into(), json!({"id": id.0, "start": p, "end": base.len() - s, "runs": runs})));
        self.note_edit(fresh, id, (&base, base.len() - s), (&cur, cur.len() - s));
        out
    }

    /// Move the caret (Shift keeps the anchor). Moving ends the typing session.
    fn move_to(&mut self, byte: usize, extend: bool) -> Vec<Action> {
        let out = self.commit();
        self.caret = byte;
        if !extend {
            self.anchor = byte;
        }
        self.listed = None;
        out
    }

    /// Begin a new edit session for `id` at `caret`, assigning a fresh session id and clearing typing,
    /// pre-edit and goal state.
    fn start_editing(&mut self, id: NodeId, caret: usize) {
        self.edit_session = next_edit_session();
        self.editing = Some(id);
        self.fresh = None;
        self.listed = None;
        self.known = None;
        self.caret = caret;
        self.anchor = caret;
        self.goal_x = None;
        self.typing = None;
        self.preedit = None;
        self.typing_ended = false;
    }

    /// Typed list detection, after a space typed on its own at the caret: when the characters
    /// from the start of the caret's paragraph up to it start a list ([`TextObject::typed_list_at`]),
    /// the typing session ends as one undo step with those characters, and `text.startList`
    /// removes them and makes the paragraph a list as the next one, so one Undo puts them back.
    fn start_typed_list(&mut self, cx: &ToolContext) -> Vec<Action> {
        let (Some(id), Some(ty)) = (self.editing, &self.typing) else { return vec![] };
        let Some(mut t) = Self::text(cx, id).cloned() else { return vec![] };
        // The document does not have the space yet (its preview is applied after this input).
        t.runs = ty.cur.clone();
        let at = self.caret;
        let Some((start, list)) = t.typed_list_at(at) else { return vec![] };
        let mut out = self.commit();
        out.push(Action::Exec("text.startList".into(), json!({"id": id.0, "at": at})));
        self.caret = start;
        self.anchor = start;
        let text = t.plain_text();
        if let Some(listed) = text.get(..start).zip(text.get(at..)).map(|(a, b)| format!("{a}{b}")) {
            self.note_edit(true, id, (&text, at), (&listed, start));
        }
        self.listed = Some(TypedList { id, start, end: at, text, list });
        out
    }

    /// Note the edited object's text as it is in the document now.
    fn remember(&mut self, cx: &ToolContext) {
        self.known = self.editing.and_then(|id| Some((id, Self::text(cx, id)?.plain_text())));
    }

    /// After a command from outside the tool: unless it was Undo or Redo, a command that changed
    /// the edited text recorded a step, which drops the steps Undo took back from Redo, so the tool
    /// drops them too and notes the step. A command that leaves the text as it was (a query) keeps
    /// them.
    fn after_outside_command(&mut self, cx: &ToolContext) {
        let stepped = std::mem::take(&mut self.stepped);
        let known = self.known.take();
        self.remember(cx);
        if stepped || known == self.known {
            return;
        }
        self.edits.retain(|e| e.done);
        if let (Some((was, old)), Some((id, new))) = (&known, &self.known)
            && was == id
        {
            let e = OwnEdit { id: *id, before: text_hash(old), after: text_hash(new), ends: None, done: true };
            self.push_edit(e);
        }
    }

    /// Note an undo step of the tool's own in `id`: the text before it and after it, each with the
    /// byte where the changed span ends. `fresh` starts a new step; otherwise the step the tool
    /// noted last grows (the typing session goes on). A new step drops the steps Undo took back,
    /// as the history drops them from Redo.
    fn note_edit(&mut self, fresh: bool, id: NodeId, before: (&str, usize), after: (&str, usize)) {
        let e = OwnEdit { id, before: text_hash(before.0), after: text_hash(after.0), ends: Some((before.1, after.1)), done: true };
        self.edits.retain(|e| e.done);
        match self.edits.last_mut() {
            Some(last) if !fresh && last.id == id && last.ends.is_some() => *last = e,
            _ => self.push_edit(e),
        }
        self.known = Some((id, after.0.to_string()));
    }

    fn push_edit(&mut self, e: OwnEdit) {
        if self.edits.len() >= OWN_EDITS_MAX {
            self.edits.remove(0);
        }
        self.edits.push(e);
    }

    /// After Undo or Redo, the caret goes to the end of the span the step changed in the edited
    /// text, with nothing selected. A step that leaves the text as it was (a formatting change)
    /// leaves the caret and the selection where they are, within the text.
    fn after_history_step(&mut self, cx: &ToolContext, undo: bool) {
        self.stepped = true;
        let Some(id) = self.editing else { return };
        let Some(text) = Self::text(cx, id).map(TextObject::plain_text) else { return };
        match self.known.take() {
            Some((known, old)) if known == id && old != text => {
                let (old_hash, new_hash) = (text_hash(&old), text_hash(&text));
                // Undo takes back the object's last step still made that changed its text, and Redo
                // makes again the one Undo took back last, when its texts match (a step that left
                // the text as it was does not come here). A step the tool made knows where its
                // span ends. Otherwise the text after the changed span is the common suffix (a
                // removal's span is empty).
                let changed = |e: &&mut OwnEdit| e.id == id && e.before != e.after;
                let step = if undo {
                    self.edits.iter_mut().rev().filter(|e| e.done).find(changed).filter(|e| (e.after, e.before) == (old_hash, new_hash))
                } else {
                    self.edits.iter_mut().filter(|e| !e.done).find(changed).filter(|e| (e.before, e.after) == (old_hash, new_hash))
                };
                let own = step.and_then(|e| {
                    e.done = !undo;
                    e.ends.map(|(before, after)| if undo { before } else { after })
                });
                self.caret = match own {
                    Some(end) => edit::floor_char(&text, end),
                    None => text.len() - common_affixes(&old, &text, usize::MAX).1,
                };
                self.anchor = self.caret;
                self.goal_x = None;
            }
            _ => {
                self.caret = edit::floor_char(&text, self.caret);
                self.anchor = edit::floor_char(&text, self.anchor);
            }
        }
        self.known = Some((id, text));
    }

    /// A `command.batch` began (`batch.begin`), ended (`batch.end`), or failed and was rolled back
    /// (`batch.rollBack`). After a rollback, what the batch's Undo and Redo steps changed in the tool
    /// is as it was at the batch's start, when the tool still edits the same object.
    fn batch_notice(&mut self, what: &str) {
        match what {
            "batch.begin" => self.batches.push(HistoryState {
                editing: self.editing,
                caret: self.caret,
                anchor: self.anchor,
                goal_x: self.goal_x,
                known: self.known.clone(),
                edits: self.edits.clone(),
                stepped: self.stepped,
            }),
            "batch.end" => {
                self.batches.pop();
            }
            "batch.rollBack" => {
                if let Some(s) = self.batches.pop()
                    && s.editing == self.editing
                {
                    (self.caret, self.anchor, self.goal_x) = (s.caret, s.anchor, s.goal_x);
                    (self.known, self.edits, self.stepped) = (s.known, s.edits, s.stepped);
                }
            }
            _ => {}
        }
    }

    /// After Undo of a list that typed characters started (and of what was typed after it), the
    /// text is as it was typed: the caret goes on after the characters. After Redo the text is as
    /// the list left it: the caret goes back to the paragraph's start.
    fn after_typed_list(&mut self, cx: &ToolContext) {
        let Some(l) = &self.listed else { return };
        if self.editing != Some(l.id) || self.typing.is_some() || self.caret != self.anchor {
            return;
        }
        let Some(t) = Self::text(cx, l.id) else { return };
        let (text, list) = (t.plain_text(), t.para_at(t.paragraphs_in(l.start, l.start).start).list);
        let listed_text = l.text.get(..l.start).zip(l.text.get(l.end..)).map(|(a, b)| format!("{a}{b}"));
        if text == l.text && list.is_none() {
            self.caret = l.end;
        } else if self.caret == l.end && Some(&text) == listed_text.as_ref() && list == Some(l.list) {
            self.caret = l.start;
        }
        self.anchor = self.caret;
    }

    /// Byte under `p` in the edited text if `p` is inside it (frame, path band or layout bounds).
    fn hit_edited(&self, cx: &ToolContext, p: Point) -> Option<usize> {
        let t = self.current(cx)?;
        let lay = self.layout(&t);
        let local = t.xf.inverse() * p;
        let tol = cx.tol(4.0);
        let inside = match &t.kind {
            TextKind::Area { frame } => frame.bounds().is_some_and(|b| b.inflate(tol, tol).contains(local)),
            _ => lay.bounds.inflate(tol, tol).contains(local),
        };
        inside.then(|| vectorcraft_text::hit_byte(&lay, local))
    }

    fn select_word_or_para(&mut self, cx: &ToolContext, para: bool) {
        let s = self.plain(cx);
        let r = if para { edit::paragraph_at(&s, self.caret) } else { edit::word_at(&s, self.caret) };
        self.anchor = r.start;
        self.caret = r.end;
    }

    /// Path under `p` for the Area Type / Type on a Path tools.
    fn path_at(cx: &ToolContext, p: Point, closed: bool) -> Option<NodeId> {
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { tol: cx.tol(4.0), outline: true, ..Default::default() })?;
        match &cx.doc.node(h.leaf)?.kind {
            NodeKind::Path { path, .. } if !closed || path.is_closed() => Some(h.leaf),
            _ => None,
        }
    }

    /// Start editing the type under `p` with the caret there → the actions (ending the previous
    /// edit, selecting the type); None when no type is under `p`. A click among the characters
    /// edits them, whatever Type Object Selection by Path Only says.
    fn edit_at(&mut self, cx: &ToolContext, p: Point) -> Option<Vec<Action>> {
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { type_path_only: false, ..cx.hit_options() })?;
        let Some(NodeKind::Text(t)) = cx.doc.node(h.leaf).map(|n| &n.kind) else { return None };
        let lay = self.layout(t);
        let byte = vectorcraft_text::hit_byte(&lay, t.xf.inverse() * p);
        let mut out = self.finish(cx);
        self.start_editing(h.leaf, byte);
        self.remember(cx);
        self.clicks = (Some(p), 1);
        out.push(Action::Exec("select.set".into(), json!({"ids": [h.leaf.0]})));
        Some(out)
    }

    /// The button released after a press at `start` (new type goes at `at`). While type is being
    /// edited, a click that lands on no type only ends the editing; the next click makes new type
    /// (#2871). A drag that draws an area-type frame makes it either way.
    fn on_up(&mut self, cx: &ToolContext, start: Point, at: Point) -> Vec<Action> {
        let (mode, vertical) = self.held.take().unwrap_or((self.mode, self.vertical));
        // Click into existing text: place the caret.
        if let Some(out) = self.edit_at(cx, start) {
            return out;
        }
        let was_editing = self.editing.is_some_and(|id| Self::text(cx, id).is_some());
        let mut out = self.finish(cx);
        let drag = self.drag.take();
        // New type is set along General › Constrain Angle's axes (none at 0° or a whole turn): an
        // area's box measured along them, turned about its top-left corner, point type about its
        // first baseline's start.
        let axes = vectorcraft_geom::constrain_axes(cx.constrain_angle);
        let back = axes.map(|a| a.inverse());
        let area = drag
            .map(|d| match back {
                Some(b) => Rect::from_points(b * at, b * d),
                None => Rect::from_points(at, d),
            })
            .filter(|r| r.width() > cx.tol(6.0) && r.height() > cx.tol(6.0));
        if was_editing && area.is_none() {
            return out;
        }
        // Area Type / Type on a Path: click a path.
        if mode != Mode::Type && drag.is_none() {
            let on_path = mode == Mode::OnPath;
            if let Some(pid) = Self::path_at(cx, start, !on_path) {
                let mode = if on_path { "onPath" } else { "area" };
                out.push(Action::Exec(
                    "text.createInPath".into(),
                    json!({"path": pid.0, "mode": mode, "text": "", "at": [start.x, start.y], "vertical": vertical, "placeholder": cx.placeholder_text}),
                ));
                out.push(Action::Notify("text.editNew".into()));
                return out;
            }
        }
        let mut params = match area {
            Some(r) => {
                let o = axes.map_or(r.origin(), |a| a * r.origin());
                json!({"x": o.x, "y": o.y, "text": "", "area": {"width": r.width(), "height": r.height()}})
            }
            None => json!({"x": at.x, "y": at.y, "text": ""}),
        };
        if let Some(rotation) = vectorcraft_geom::constrain_rotation(cx.constrain_angle) {
            params["rotation"] = json!(rotation);
        }
        params["vertical"] = json!(vertical);
        params["placeholder"] = json!(cx.placeholder_text);
        out.push(Action::Exec("text.create".into(), params));
        out.push(Action::Notify("text.editNew".into()));
        out
    }

    /// Delete plain-text byte ranges in one typing session, shifting the caret, anchor and any
    /// pre-edit range so the selection's direction and the marked text are preserved.
    fn delete_ranges(&mut self, cx: &ToolContext, ranges: &[(usize, usize)]) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        let Some(t) = Self::text(cx, id) else {
            self.editing = None;
            return vec![];
        };
        let mut out = vec![];
        // The document changed under us (undo, a panel edit): start a new session.
        if self.typing.as_ref().is_some_and(|ty| ty.cur != t.runs) {
            out.push(Action::Commit);
            self.typing = None;
            // The marked text belonged to the replaced session; it must not be sliced out of the
            // new document text by a stale range.
            self.preedit = None;
        }
        let fresh = self.typing.is_none();
        if fresh {
            out.push(Action::Begin("Typing".into()));
            self.typing_ended = false;
        }
        let len = match &self.typing {
            Some(ty) => edit::runs_len(&ty.cur),
            None => edit::runs_len(&t.runs),
        };
        let mut sorted: Vec<(usize, usize)> = ranges.iter().map(|&(a, b)| (a.min(b).min(len), a.max(b).min(len))).filter(|(a, b)| a < b).collect();
        sorted.sort_unstable();
        if sorted.is_empty() {
            return out;
        }
        // Shift the selection ends and the pre-edit by the bytes removed before them. The ranges
        // lie outside the selection/pre-edit, so both ends move by the same preceding amount and
        // a reversed selection stays reversed.
        let shift = |at: usize| -> usize {
            let removed: usize = sorted.iter().map(|&(a, b)| if at >= b { b - a } else { at.saturating_sub(a) }).sum();
            at.saturating_sub(removed)
        };
        let (caret, anchor) = (self.caret.min(len), self.anchor.min(len));
        self.caret = shift(caret);
        self.anchor = shift(anchor);
        if let Some(p) = &mut self.preedit {
            p.range.start = shift(p.range.start.min(len));
            p.range.end = shift(p.range.end.min(len));
        }
        self.goal_x = None;
        let ty = self.typing.get_or_insert_with(|| Typing { base: t.runs.clone(), cur: t.runs.clone(), lo: usize::MAX });
        // Delete from the end so earlier offsets stay valid; each call floors to a char boundary
        // and the one Preview below still makes the whole session one undo step.
        for &(a, b) in sorted.iter().rev() {
            ty.lo = ty.lo.min(a);
            edit::replace_range(&mut ty.cur, a, b, "");
        }
        // One preview for the whole session, exactly as `replace` does.
        let base: String = ty.base.iter().map(|r| r.text.as_str()).collect();
        let cur: String = ty.cur.iter().map(|r| r.text.as_str()).collect();
        let (p, s) = common_affixes(&base, &cur, ty.lo);
        let runs = edit::slice_runs(&ty.cur, p, cur.len() - s);
        let runs = if runs.is_empty() { json!([]) } else { serde_json::to_value(&runs).unwrap_or(json!([])) };
        out.push(Action::Preview("text.editRange".into(), json!({"id": id.0, "start": p, "end": base.len() - s, "runs": runs})));
        self.note_edit(fresh, id, (&base, base.len() - s), (&cur, cur.len() - s));
        out
    }
}

#[cfg(test)]
mod tests;
