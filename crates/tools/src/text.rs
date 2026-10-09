//! The Type tools: Type (T), Area Type and Type on a Path.
//!
//! Type: click places point type, drag draws an area-type frame, clicking into existing text
//! places the caret. Area Type / Type on a Path: click a path to turn it into a text frame or a
//! baseline (`text.createInPath`). Point type placed by a click and left empty is discarded when
//! editing ends (`text.discardEmpty`); frames keep their shape. Where new type goes (the click, the
//! frame's corners) snaps to Smart Guides ([`DrawSnap`]), hovering too.
//!
//! While editing: caret movement by character / word (Cmd or Alt) / line (Up/Down) / line ends
//! (Home/End; Cmd = whole text), Shift extends the selection, drag selects, double-click selects a
//! word and triple-click a paragraph, typing replaces the selection. Edits are coalesced into one
//! undo step per typing session: the tool keeps the styled runs locally and previews a single
//! `text.editRange` (the changed span, with its styled runs) against the session snapshot.
//! Styling a selected range goes through `text.setRangeStyle` (the Character panel reads the
//! selection from [`Tool::options`]); Alt+arrows step it by the Preferences › Type increments
//! (`type.step`). New type starts with placeholder text, selected, when Fill New Type Objects With
//! Placeholder Text is on.
//!
//! IME: the marked text of a composition is part of the typing session (so it lays out in place),
//! underlined, until the IME commits it (it then goes through [`Tool::text_input`]) or clears it.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::hit::hit_test;
use vectorcraft_doc::{NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{BezPath, Point, Rect, Shape};
use vectorcraft_text::{FontDb, TextLayout, edit};

use crate::guides::DrawSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

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

#[derive(Default)]
pub struct TypeTool {
    vertical: bool,
    mode: Mode,
    /// Text object being edited.
    editing: Option<NodeId>,
    /// The point type a click just placed (edited now): discarded if empty when editing ends.
    fresh: Option<NodeId>,
    /// Caret and selection anchor (byte offsets into the plain text).
    caret: usize,
    anchor: usize,
    /// Horizontal position kept by Up/Down (text space).
    goal_x: Option<f64>,
    typing: Option<Typing>,
    preedit: Option<Preedit>,
    /// Where the button went down, and where new type goes from there (snapped to Smart Guides).
    press: Option<(Point, Point)>,
    /// The other corner of an area dragged out (snapped).
    drag: Option<Point>,
    /// Smart Guides for where new type goes.
    snap: DrawSnap,
    /// Existing, unedited type under the pointer, shown with an edit cue.
    hover_edit: Option<Point>,
    /// The press landed in the edited text: dragging selects.
    selecting: bool,
    /// Click counting for double/triple click (position of the last click, count).
    clicks: (Option<Point>, u8),
    /// Styled copy of the last copied range (pasting the same plain text keeps its styles).
    clipboard: Vec<TextRun>,
    cache: Mutex<Option<(TextObject, Arc<TextLayout>)>>,
}

impl TypeTool {
    pub fn new(id: &str) -> Self {
        let mode = match id {
            "areaType" | "verticalAreaType" => Mode::Area,
            "typeOnPath" | "verticalTypeOnPath" => Mode::OnPath,
            _ => Mode::Type,
        };
        Self { mode, vertical: id.starts_with("vertical"), ..Default::default() }
    }

    fn text<'a>(cx: &'a ToolContext, id: NodeId) -> Option<&'a TextObject> {
        match &cx.doc.node(id)?.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        }
    }

    fn text_at(cx: &ToolContext, p: Point) -> Option<NodeId> {
        let h = hit_test(cx.doc, p, vectorcraft_doc::hit::HitOptions { type_path_only: false, ..cx.hit_options() })?;
        Self::text(cx, h.leaf)?;
        Some(h.leaf)
    }

    /// Layout of `t`, cached by content (overlays run every frame).
    fn layout(&self, t: &TextObject) -> Arc<TextLayout> {
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, l)) = c.as_ref()
            && k == t
        {
            return l.clone();
        }
        let l = Arc::new(vectorcraft_text::layout(FontDb::global(), t));
        *c = Some((t.clone(), l.clone()));
        l
    }

    fn plain(&self, cx: &ToolContext) -> String {
        match &self.typing {
            Some(ty) => ty.cur.iter().map(|r| r.text.as_str()).collect(),
            None => self.editing.and_then(|id| Self::text(cx, id)).map(|t| t.plain_text()).unwrap_or_default(),
        }
    }

    /// The edited object as it currently is (with the typing session's runs).
    fn current(&self, cx: &ToolContext) -> Option<TextObject> {
        let mut t = Self::text(cx, self.editing?)?.clone();
        if let Some(ty) = &self.typing {
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
        }
        if self.typing.is_none() {
            out.push(Action::Begin("Typing".into()));
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
        out
    }

    /// Move the caret (Shift keeps the anchor). Moving ends the typing session.
    fn move_to(&mut self, byte: usize, extend: bool) -> Vec<Action> {
        let out = self.commit();
        self.caret = byte;
        if !extend {
            self.anchor = byte;
        }
        out
    }

    fn start_editing(&mut self, id: NodeId, caret: usize) {
        self.editing = Some(id);
        self.fresh = None;
        self.caret = caret;
        self.anchor = caret;
        self.goal_x = None;
        self.typing = None;
        self.preedit = None;
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
        let id = Self::text_at(cx, p)?;
        let t = Self::text(cx, id)?;
        let lay = self.layout(t);
        let byte = vectorcraft_text::hit_byte(&lay, t.xf.inverse() * p);
        let mut out = self.finish(cx);
        self.start_editing(id, byte);
        self.clicks = (Some(p), 1);
        out.push(Action::Exec("select.set".into(), json!({"ids": [id.0]})));
        Some(out)
    }

    /// The button released after a press at `start` (new type goes at `at`).
    fn on_up(&mut self, cx: &ToolContext, start: Point, at: Point) -> Vec<Action> {
        // Click into existing text: place the caret.
        if let Some(out) = self.edit_at(cx, start) {
            return out;
        }
        let mut out = self.finish(cx);
        let drag = self.drag.take();
        // Area Type / Type on a Path: click a path.
        if self.mode != Mode::Type && drag.is_none() {
            let on_path = self.mode == Mode::OnPath;
            if let Some(pid) = Self::path_at(cx, start, !on_path) {
                let mode = if on_path { "onPath" } else { "area" };
                out.push(Action::Exec(
                    "text.createInPath".into(),
                    json!({"path": pid.0, "mode": mode, "text": "", "at": [start.x, start.y], "vertical": self.vertical, "placeholder": cx.placeholder_text}),
                ));
                out.push(Action::Notify("text.editNew".into()));
                return out;
            }
        }
        let area = drag.map(|d| Rect::from_points(at, d)).filter(|r| r.width() > cx.tol(6.0) && r.height() > cx.tol(6.0));
        let mut params = match area {
            Some(r) => json!({"x": r.x0, "y": r.y0, "text": "", "area": {"width": r.width(), "height": r.height()}}),
            None => json!({"x": at.x, "y": at.y, "text": ""}),
        };
        params["vertical"] = json!(self.vertical);
        params["placeholder"] = json!(cx.placeholder_text);
        out.push(Action::Exec("text.create".into(), params));
        out.push(Action::Notify("text.editNew".into()));
        out
    }

    fn overflow_marker(t: &TextObject, lay: &TextLayout, cx: &ToolContext, o: &mut Vec<Overlay>) {
        if !lay.overflow {
            return;
        }
        // The out port: bottom-right of the frame (end of the path for type on a path).
        let port = match &t.kind {
            TextKind::Area { frame } => frame.bounds().map(|b| t.xf * Point::new(b.x1, b.y1)),
            TextKind::OnPath { path, .. } => path.to_bezpath().segments().last().map(|s| t.xf * vectorcraft_geom::ParamCurve::end(&s)),
            TextKind::Point => None,
        };
        let Some(c) = port else { return };
        let h = cx.tol(4.5);
        let r = Rect::new(c.x - h, c.y - h, c.x + h, c.y + h);
        let red = [230, 30, 30];
        o.push(Overlay::Path { path: r.to_path(0.1), color: red, width: 1.0, dashed: false });
        let (k, m) = (cx.tol(2.5), r.center());
        o.push(Overlay::Line { a: Point::new(m.x - k, m.y), b: Point::new(m.x + k, m.y), color: red, dashed: false });
        o.push(Overlay::Line { a: Point::new(m.x, m.y - k), b: Point::new(m.x, m.y + k), color: red, dashed: false });
    }
}

/// Byte range in `s` of the characters `r` (IME ranges count characters; carets count bytes).
/// `None` when `r` runs past the end of `s` or backwards.
/// The `type.step` an Alt+arrow asks for, by one step: ←/→ kerning at a `caret`, else tracking;
/// ↑/↓ leading (down opens it up); Shift+↑/↓ baseline shift.
fn step_key(key: ToolKey, shift: bool, caret: bool) -> Option<(&'static str, f64)> {
    let sign = |up: bool| if up { 1.0 } else { -1.0 };
    Some(match (key, shift) {
        (ToolKey::Left | ToolKey::Right, false) => (if caret { "kerning" } else { "tracking" }, sign(key == ToolKey::Right)),
        (ToolKey::Up | ToolKey::Down, false) => ("leading", sign(key == ToolKey::Down)),
        (ToolKey::Up | ToolKey::Down, true) => ("baselineShift", sign(key == ToolKey::Up)),
        _ => return None,
    })
}

fn char_range_to_bytes(s: &str, r: Range<usize>) -> Option<Range<usize>> {
    let byte = |c: usize| if c == s.chars().count() { Some(s.len()) } else { s.char_indices().nth(c).map(|(i, _)| i) };
    let (a, b) = (byte(r.start)?, byte(r.end)?);
    (a <= b).then_some(a..b)
}

/// Lengths of the common prefix (at most `max_prefix`) and suffix of `a` and `b` (on char
/// boundaries, not overlapping).
fn common_affixes(a: &str, b: &str, max_prefix: usize) -> (usize, usize) {
    let mut p = 0;
    for ((i, x), y) in a.char_indices().zip(b.chars()) {
        if x != y || i + x.len_utf8() > max_prefix {
            break;
        }
        p = i + x.len_utf8();
    }
    let max_s = (a.len() - p).min(b.len() - p);
    let mut s = 0;
    for (x, y) in a[p..].chars().rev().zip(b[p..].chars().rev()) {
        if x != y || s + x.len_utf8() > max_s {
            break;
        }
        s += x.len_utf8();
    }
    (p, s)
}

impl Tool for TypeTool {
    fn id(&self) -> &'static str {
        match (self.mode, self.vertical) {
            (Mode::Type, false) => "type",
            (Mode::Type, true) => "verticalType",
            (Mode::Area, false) => "areaType",
            (Mode::Area, true) => "verticalAreaType",
            (Mode::OnPath, false) => "typeOnPath",
            (Mode::OnPath, true) => "verticalTypeOnPath",
        }
    }
    fn busy(&self) -> bool {
        self.press.is_some() || self.selecting
    }
    fn wants_text(&self) -> bool {
        self.editing.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.hover_edit = None;
                if let Some(byte) = self.hit_edited(cx, ev.pos) {
                    let same = self.clicks.0.is_some_and(|q| (q - ev.pos).hypot() <= cx.tol(4.0));
                    let n = if same { self.clicks.1 + 1 } else { 1 };
                    self.clicks = (Some(ev.pos), n);
                    let out = self.move_to(byte, ev.mods.shift);
                    self.goal_x = None;
                    if n >= 3 {
                        self.select_word_or_para(cx, true);
                        self.clicks.1 = 0;
                    } else {
                        self.selecting = true;
                    }
                    return out;
                }
                self.clicks = (None, 0);
                // A press on other type edits it from there, so a drag selects its text at once,
                // as it does in the type being edited.
                if let Some(out) = self.edit_at(cx, ev.pos) {
                    self.selecting = true;
                    return out;
                }
                self.press = Some((ev.pos, self.snap.press(cx, ev.pos, self.editing.as_slice(), None)));
                self.drag = None;
                vec![]
            }
            PointerKind::Drag => {
                if self.selecting {
                    if let Some(t) = self.current(cx) {
                        let lay = self.layout(&t);
                        self.caret = vectorcraft_text::hit_byte(&lay, t.xf.inverse() * ev.pos);
                        self.clicks = (None, 0);
                    }
                } else if self.press.is_some() {
                    self.drag = Some(self.snap.drag(cx, ev.pos, None));
                }
                vec![]
            }
            PointerKind::Up => {
                if self.selecting {
                    self.selecting = false;
                    return vec![];
                }
                self.snap.clear();
                let Some((start, at)) = self.press.take() else { return vec![] };
                self.on_up(cx, start, at)
            }
            PointerKind::DoubleClick => {
                if self.editing.is_some() && self.hit_edited(cx, ev.pos).is_some() {
                    self.select_word_or_para(cx, false);
                    self.clicks = (Some(ev.pos), 2);
                }
                vec![]
            }
            PointerKind::Move => {
                self.hover_edit = Self::text_at(cx, ev.pos).filter(|id| Some(*id) != self.editing).map(|_| ev.pos);
                // The type being edited is no target: its guides would only point at itself.
                self.snap.hover(cx, ev.pos, self.editing.as_slice(), None);
                vec![]
            }
        }
    }
    fn text_input(&mut self, cx: &ToolContext, s: &str) -> Vec<Action> {
        if self.editing.is_none() {
            return vec![];
        }
        let s: String = s.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).map(|c| if c == '\r' { '\n' } else { c }).collect();
        // An IME commit replaces its marked text (an empty commit just removes it).
        let committing = self.preedit.is_some();
        if committing && s.is_empty() {
            return self.ime_preedit(cx, "", None);
        }
        if let Some(p) = self.preedit.take() {
            self.anchor = p.range.start;
            self.caret = p.range.end;
        }
        if s.is_empty() {
            return vec![];
        }
        // Use Typographer's Quotes (Document Setup): a typed straight quote becomes the document's
        // opening or closing quote, by the character before it.
        let setup = &cx.doc.setup;
        let s = if setup.typographers_quotes && matches!(s.as_str(), "\"" | "'") {
            let text = self.plain(cx);
            let mut prev = text.get(..self.caret.min(self.anchor)).and_then(|b| b.chars().next_back());
            setup.quotes.apply(&s, &mut prev)
        } else {
            s
        };
        self.clicks = (None, 0);
        // Pasting what we copied keeps its formatting (unless pasted text is kept plain).
        let clip: String = self.clipboard.iter().map(|r| r.text.as_str()).collect();
        if !committing && s.len() > 1 && s == clip && !cx.paste_plain_text {
            let runs = self.clipboard.clone();
            return self.replace(cx, "", Some(runs));
        }
        self.replace(cx, &s, None)
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, mods: Mods) -> Vec<Action> {
        // The IME owns the keys while it composes (the UI doesn't send them; this is a guard).
        if self.preedit.is_some() {
            return vec![];
        }
        let Some(t) = self.current(cx) else {
            self.editing = None;
            return vec![];
        };
        let text = t.plain_text();
        let len = text.len();
        self.caret = self.caret.min(len);
        self.anchor = self.anchor.min(len);
        let (a, b) = self.sel();
        // Alt+arrows step the type by the Preferences › Type increments (Cmd/Ctrl too: five steps).
        if mods.alt
            && let Some(id) = self.editing
            && let Some((attribute, by)) = step_key(key, mods.shift, a == b)
        {
            let mut out = self.commit();
            let by = if mods.cmd { by * 5.0 } else { by };
            out.push(Action::Exec("type.step".into(), json!({"id": id.0, "start": a, "end": b, "attribute": attribute, "by": by})));
            return out;
        }
        let word = mods.cmd || mods.alt;
        let lay = self.layout(&t);
        let key = if lay.vertical {
            match key {
                ToolKey::Up => ToolKey::Left,
                ToolKey::Down => ToolKey::Right,
                ToolKey::Right => ToolKey::Up,
                ToolKey::Left => ToolKey::Down,
                other => other,
            }
        } else {
            key
        };
        let vertical = matches!(key, ToolKey::Up | ToolKey::Down);
        if !vertical {
            self.goal_x = None;
        }
        match key {
            ToolKey::Escape => self.finish(cx),
            ToolKey::Enter => self.replace(cx, "\n", None),
            ToolKey::Tab => self.replace(cx, "\t", None),
            ToolKey::Backspace | ToolKey::Delete if a != b => self.replace(cx, "", None),
            ToolKey::Backspace if self.caret > 0 => {
                self.anchor = if word { edit::prev_word(&text, self.caret) } else { edit::prev_char(&text, self.caret) };
                self.replace(cx, "", None)
            }
            ToolKey::Delete if self.caret < len => {
                self.anchor = if word { edit::next_word(&text, self.caret) } else { edit::next_char(&text, self.caret) };
                self.replace(cx, "", None)
            }
            ToolKey::Left => {
                let to = if a != b && !mods.shift {
                    if lay.glyphs.iter().any(|g| g.rtl)
                        && vectorcraft_text::caret_position(&lay, a).0.x > vectorcraft_text::caret_position(&lay, b).0.x
                    {
                        b
                    } else {
                        a
                    }
                } else if word {
                    edit::prev_word(&text, self.caret)
                } else {
                    if lay.glyphs.iter().any(|g| g.rtl) {
                        vectorcraft_text::caret_horizontal(&lay, self.caret, false)
                    } else {
                        edit::prev_char(&text, self.caret)
                    }
                };
                self.move_to(to, mods.shift)
            }
            ToolKey::Right => {
                let to = if a != b && !mods.shift {
                    if lay.glyphs.iter().any(|g| g.rtl)
                        && vectorcraft_text::caret_position(&lay, a).0.x > vectorcraft_text::caret_position(&lay, b).0.x
                    {
                        a
                    } else {
                        b
                    }
                } else if word {
                    edit::next_word(&text, self.caret)
                } else {
                    if lay.glyphs.iter().any(|g| g.rtl) {
                        vectorcraft_text::caret_horizontal(&lay, self.caret, true)
                    } else {
                        edit::next_char(&text, self.caret)
                    }
                };
                self.move_to(to, mods.shift)
            }
            ToolKey::Up | ToolKey::Down => {
                let d = if key == ToolKey::Up { -1 } else { 1 };
                let from = if a != b && !mods.shift { if d < 0 { a } else { b } } else { self.caret };
                let x = self.goal_x.unwrap_or_else(|| lay.logical_point(vectorcraft_text::caret_position(&lay, from).0).x);
                let to = if mods.cmd {
                    // Cmd+Up/Down: paragraph start / end.
                    let p = edit::paragraph_at(&text, from);
                    if d < 0 {
                        if from == p.start { edit::paragraph_at(&text, p.start.saturating_sub(1)).start } else { p.start }
                    } else if from == p.end {
                        edit::paragraph_at(&text, (p.end + 1).min(len)).end
                    } else {
                        p.end
                    }
                } else {
                    vectorcraft_text::caret_vertical(&lay, from, d, x)
                };
                let out = self.move_to(to, mods.shift);
                self.goal_x = Some(x);
                out
            }
            ToolKey::Home => {
                let to = if mods.cmd { 0 } else { vectorcraft_text::line_home(&lay, self.caret) };
                self.move_to(to, mods.shift)
            }
            ToolKey::End => {
                let to = if mods.cmd { len } else { vectorcraft_text::line_end_of(&lay, self.caret) };
                self.move_to(to, mods.shift)
            }
            _ => vec![],
        }
    }
    fn ime_preedit(&mut self, cx: &ToolContext, text: &str, active_chars: Option<Range<usize>>) -> Vec<Action> {
        if self.editing.is_none() {
            return vec![];
        }
        // A clear with nothing marked (some integrations send one when the IME is enabled) must
        // not delete the selection.
        if text.is_empty() && self.preedit.is_none() {
            return vec![];
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        // The new marked text replaces the previous one, or the selection when it starts.
        if let Some(p) = self.preedit.take() {
            self.anchor = p.range.start;
            self.caret = p.range.end;
        }
        self.clicks = (None, 0);
        let mut out = self.replace(cx, &text, None);
        if text.is_empty() {
            // Cancelled before anything else was typed: no empty undo step.
            if self.typing.as_ref().is_some_and(|ty| ty.cur == ty.base) {
                self.typing = None;
                out.push(Action::Cancel);
            }
            return out;
        }
        let end = self.caret;
        let start = end.saturating_sub(text.len());
        self.preedit = Some(Preedit { range: start..end, active: active_chars.and_then(|r| char_range_to_bytes(&text, r)) });
        out
    }
    fn composing(&self) -> bool {
        self.preedit.is_some()
    }
    fn ime_caret(&self, cx: &ToolContext) -> Option<(Point, Point)> {
        let t = self.current(cx)?;
        let lay = self.layout(&t);
        let len = t.plain_text().len();
        // The candidate window follows the clause being converted, else the caret.
        let at = match &self.preedit {
            Some(p) => p.range.start + p.active.as_ref().map_or(0, |r| r.start),
            None => self.caret.min(self.anchor),
        };
        let (a, b) = vectorcraft_text::caret_position(&lay, at.min(len));
        Some((t.xf * a, t.xf * b))
    }
    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.finish(cx)
    }
    /// Editing ends when a command takes the text out of the selection (Deselect, another object
    /// picked in the Layers panel) or removes it.
    fn after_command(&mut self, cx: &ToolContext) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        let mut cur = Some(id).filter(|i| cx.doc.node(*i).is_some());
        while let Some(c) = cur {
            if cx.selection.contains(c) {
                return vec![];
            }
            cur = cx.doc.parent_of(c);
        }
        self.finish(cx)
    }
    /// After `text.create` / `text.createInPath` the engine selects the new object; edit it.
    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "text.editNew"
            && let Some(id) = cx.selection.objects.first().copied()
        {
            self.start_editing(id, 0);
            let t = Self::text(cx, id);
            self.fresh = t.filter(|t| matches!(t.kind, TextKind::Point)).map(|_| id);
            // Placeholder text comes selected: typing replaces it.
            self.caret = t.map_or(0, |t| edit::runs_len(&t.runs));
        }
    }
    /// `{editing, start, end, caret, anchor, typing, composing}` — the Character panel styles
    /// `start..end`.
    fn options(&self) -> Value {
        match self.editing {
            Some(id) => {
                // Offsets may exceed the text after Select All; callers clamp to the text length.
                let (a, b) = self.sel();
                json!({"editing": id.0, "start": a, "end": b, "caret": self.caret, "anchor": self.anchor, "typing": self.typing.is_some(), "composing": self.preedit.is_some()})
            }
            None => json!({"editing": null}),
        }
    }
    /// `select {start, end}` / `selectAll` / `copy` (with `runs` of the selection) /
    /// `commitTyping` (the caller commits the engine interaction).
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "commitTyping" | "endTyping" => {
                self.typing = None;
                self.preedit = None;
            }
            "selectAll" if self.editing.is_some() => {
                self.anchor = 0;
                self.caret = usize::MAX / 4;
            }
            "select" if self.editing.is_some() => {
                let g = |k: &str| v.get(k).and_then(Value::as_u64).map(|x| x as usize);
                if let (Some(a), Some(b)) = (g("start"), g("end")) {
                    self.anchor = a;
                    self.caret = b;
                }
            }
            "copy" => {
                if let Ok(runs) = serde_json::from_value::<Vec<TextRun>>(v.clone()) {
                    self.clipboard = runs;
                }
            }
            "stopEditing" => {
                self.typing = None;
                self.preedit = None;
                self.editing = None;
            }
            _ => {}
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let Some(p) = self.hover_edit {
            o.push(Overlay::Label { p: Point::new(p.x + cx.tol(8.0), p.y - cx.tol(8.0)), text: "Click to edit text".into(), color: [79, 128, 255] });
        }
        if let (Some((_, s)), Some(d)) = (self.press, self.drag) {
            o.push(Overlay::Marquee(Rect::from_points(s, d)));
        }
        // Overflow markers on selected text.
        for id in &cx.selection.objects {
            if Some(*id) == self.editing {
                continue;
            }
            if let Some(t) = Self::text(cx, *id) {
                let lay = self.layout(t);
                Self::overflow_marker(t, &lay, cx, &mut o);
            }
        }
        let Some(t) = self.current(cx) else { return o };
        let lay = self.layout(&t);
        let len = t.plain_text().len();
        // Frame / baseline path of the edited object.
        let frame: Option<BezPath> = match &t.kind {
            TextKind::Area { frame } => Some(frame.to_bezpath()),
            TextKind::OnPath { path, .. } => Some(path.to_bezpath()),
            TextKind::Point => None,
        };
        if let Some(mut f) = frame {
            f.apply_affine(t.xf);
            o.push(Overlay::Path { path: f, color: [79, 128, 255], width: 1.0, dashed: false });
        }
        let (a, b) = (self.caret.min(self.anchor).min(len), self.caret.max(self.anchor).min(len));
        if let Some(pe) = &self.preedit {
            // Marked text: a thin underline, a thick one under the clause being converted.
            let under = |o: &mut Vec<Overlay>, r: Range<usize>, width: f32| {
                for q in vectorcraft_text::selection_quads(&lay, r.start.min(len), r.end.min(len)) {
                    let (l, rr) = (t.xf * q[3], t.xf * q[2]);
                    o.push(Overlay::Path { path: vectorcraft_geom::Line::new(l, rr).to_path(0.1), color: [0, 0, 0], width, dashed: false });
                }
            };
            under(&mut o, pe.range.clone(), 1.0);
            match &pe.active {
                Some(r) if !r.is_empty() => under(&mut o, pe.range.start + r.start..pe.range.start + r.end, 2.5),
                _ => {
                    let at = pe.range.start + pe.active.as_ref().map_or(pe.range.len(), |r| r.start);
                    let (p, q) = vectorcraft_text::caret_position(&lay, at.min(len));
                    o.push(Overlay::Line { a: t.xf * p, b: t.xf * q, color: [0, 0, 0], dashed: false });
                }
            }
        } else if a == b {
            let (p, q) = vectorcraft_text::caret_position(&lay, self.caret.min(len));
            o.push(Overlay::Line { a: t.xf * p, b: t.xf * q, color: [0, 0, 0], dashed: false });
        } else {
            for q in vectorcraft_text::selection_quads(&lay, a, b) {
                o.push(Overlay::Highlight { quad: q.map(|p| t.xf * p), color: [60, 120, 255, 90] });
            }
        }
        Self::overflow_marker(&t, &lay, cx, &mut o);
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        if self.mode != Mode::Type && Self::path_at(cx, p, self.mode == Mode::Area).is_some() {
            return Cursor::Crosshair;
        }
        Cursor::Text
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
