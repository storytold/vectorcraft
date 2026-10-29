use super::plain::{
    char_range_to_bytes, ime_surrounding_window, plain_ceil_boundary, plain_chars_after, plain_chars_before, plain_floor_boundary, plain_is_boundary,
    plain_len,
};
use super::ports::{ported, ports, ports_shown};
use super::{
    Action, BezPath, Cursor, ImeSurrounding, Mode, Mods, NodeId, Overlay, Point, PointerEvent, PointerKind, Preedit, Range, Rect, Shape,
    SurroundingUnit, TextKind, TextRun, Tool, ToolContext, ToolKey, TypeTool, Value, edit, json,
};

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
    /// Escape while editing is the tool's: it goes on to the Selection tool (#1686).
    fn claims_key(&self, _: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.editing.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
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
                self.held = Some(self.acting(ev.mods));
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
        // An IME commit replaces its marked text (an empty commit just removes it).
        let committing = self.preedit.is_some();
        // A space typed on its own at a caret may start a list.
        let list_space = !committing && s == " " && self.caret == self.anchor;
        if committing && s.is_empty() {
            return self.ime_preedit(cx, "", None);
        }
        if let Some(p) = self.preedit.take() {
            self.anchor = p.range.start;
            self.caret = p.range.end;
        }
        let Some(s) = self.input_text(cx, s) else { return vec![] };
        self.clicks = (None, 0);
        // Pasting what we copied keeps its formatting (unless pasted text is kept plain).
        let clip: String = self.clipboard.iter().map(|r| r.text.as_str()).collect();
        if !committing && s.len() > 1 && s == clip && !cx.paste_plain_text {
            let runs = self.clipboard.clone();
            return self.replace(cx, "", Some(runs));
        }
        let mut out = self.replace(cx, &s, None);
        if list_space {
            out.extend(self.start_typed_list(cx));
        }
        out
    }
    /// Would [`Tool::text_input`] change `s` before inserting it? Control characters are dropped,
    /// `\r` becomes `\n`, Typographer's Quotes rewrites a straight quote, and copied text keeps
    /// its run styles. An IME commit the application rewrites is an application edit, not an
    /// accepted IME insertion (its receipt must stay `Other`).
    fn text_input_is_verbatim(&self, cx: &ToolContext, s: &str) -> bool {
        if self.editing.is_none() {
            return true;
        }
        if s.is_empty() {
            return true;
        }
        // A straight quote's direction depends on the character before the insertion. During a
        // commit this check still sees the pre-edit at the caret, while `text_input` replaces it
        // from its start; with a custom pair whose opening quote is straight, that can look
        // verbatim here and then be rewritten. Never claim verbatim for one.
        if cx.doc.setup.typographers_quotes && matches!(s, "\"" | "'") {
            return false;
        }
        let Some(normalized) = self.input_text(cx, s) else { return false };
        if normalized != s {
            return false;
        }
        let clip: String = self.clipboard.iter().map(|r| r.text.as_str()).collect();
        !(self.preedit.is_none() && normalized.len() > 1 && normalized == clip && !cx.paste_plain_text)
    }
    /// Whether the typing session still describes the edited document; a session ended by an outside
    /// command reports false until a replacement commit starts a new one.
    fn ime_session_matches_document(&self, cx: &ToolContext) -> bool {
        match (self.editing, &self.typing) {
            (Some(id), Some(ty)) => Self::text(cx, id).is_none_or(|t| ty.cur == t.runs),
            // An outside command settled the session: the document may have changed under it, and
            // the commit that starts a new session cannot claim the IME made that change.
            (Some(_), None) => !self.typing_ended,
            _ => true,
        }
    }
    /// Handle an editing key: ignore keys while an IME composes, remap arrows in vertical text, step
    /// type attributes with Alt, and otherwise edit, navigate or end the session.
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
        if let Some(out) = self.step_attribute(key, mods, a, b) {
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
            // Escape ends editing and goes back to the tool used before double-clicking into type
            // (or to the Selection tool by default), the type selected: the keyboard is free for
            // tool shortcuts again (#1686).
            ToolKey::Escape => {
                let target = self.prev_tool.take().unwrap_or_else(|| "selection".into());
                let mut out = self.finish(cx);
                out.push(Action::SwitchTool(target));
                out
            }
            ToolKey::Enter => self.replace(cx, if mods.shift { "\u{2028}" } else { "\n" }, None),
            ToolKey::Tab => self.replace(cx, "\t", None),
            ToolKey::Backspace | ToolKey::Delete if a != b => self.replace(cx, "", None),
            ToolKey::Backspace if self.caret > 0 => {
                self.anchor = if word { edit::prev_word(&text, self.caret) } else { edit::prev_char(&text, self.caret) };
                self.replace(cx, "", None)
            }
            // Backspace takes back one character (a mistyped Thai tone mark, say); Delete, the
            // Left and Right arrows a whole cluster.
            ToolKey::Delete if self.caret < len => {
                self.anchor = if word { edit::next_word(&text, self.caret) } else { edit::next_cluster(&text, self.caret) };
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
                        vectorcraft_text::caret_horizontal_cluster(&lay, &text, self.caret, false)
                    } else {
                        edit::prev_cluster(&text, self.caret)
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
                        vectorcraft_text::caret_horizontal_cluster(&lay, &text, self.caret, true)
                    } else {
                        edit::next_cluster(&text, self.caret)
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
    /// Delete the requested bytes or characters before and after the selection or marked text,
    /// preserving the selection direction and composition and refusing a stale session; byte counts
    /// round inward to UTF-8 boundaries.
    fn ime_delete_surrounding(&mut self, cx: &ToolContext, before: usize, after: usize, unit: SurroundingUnit) -> Vec<Action> {
        let Some(id) = self.editing else { return vec![] };
        // The document changed under the session (undo, a panel edit): the ranges below would index
        // text that is no longer there. The commit that follows resets the session; do not delete
        // from the wrong text in the meantime.
        if let Some(t) = Self::text(cx, id)
            && self.typing.as_ref().is_some_and(|ty| ty.cur != t.runs)
        {
            return vec![];
        }
        // Walk the runs in place: joining the whole edited text here would copy a large document
        // for every IME deletion event, though only the characters next to the selection matter.
        let Some(segments) = self.plain_segments(cx) else { return vec![] };
        let len = plain_len(&segments);
        // The text the IME's following commit replaces (the selection, or the marked text while
        // composing). The surrounding deletion never removes it: it deletes only the bytes/characters
        // before and after it, preserving the selection direction and the composition.
        let (raw_start, raw_end) = match &self.preedit {
            Some(p) => (p.range.start.min(len), p.range.end.min(len)),
            None => (self.caret.min(self.anchor).min(len), self.caret.max(self.anchor).min(len)),
        };
        let (before_start, start, end, after_end) = match unit {
            SurroundingUnit::Bytes => {
                // Inward rounding: a count ending inside a scalar leaves that scalar whole. The
                // preceding side moves its start forward (ceil), the following side moves its end
                // backward (floor). Do not reverse these: that would delete part of a character.
                let start = plain_floor_boundary(&segments, raw_start);
                let end = plain_ceil_boundary(&segments, raw_end);
                let before_start = plain_ceil_boundary(&segments, start.saturating_sub(before));
                let after_end = plain_floor_boundary(&segments, end.saturating_add(after));
                (before_start, start, end, after_end)
            }
            SurroundingUnit::Chars => {
                let start = plain_floor_boundary(&segments, raw_start);
                let end = plain_ceil_boundary(&segments, raw_end);
                let before_start = plain_chars_before(&segments, start, before);
                let after_end = plain_chars_after(&segments, end, after);
                (before_start, start, end, after_end)
            }
        };
        let mut ranges = Vec::with_capacity(2);
        if before_start < start {
            ranges.push((before_start, start));
        }
        if end < after_end {
            ranges.push((end, after_end));
        }
        if ranges.is_empty() {
            return vec![];
        }
        self.delete_ranges(cx, &ranges)
    }
    /// Return the active Type tool's stable IME owner (document uid, target id, replacement epoch),
    /// or `None` when nothing is being edited.
    fn ime_target(&self, cx: &ToolContext) -> Option<(u64, u64, u64)> {
        let id = self.editing?;
        Self::text(cx, id)?;
        Some((cx.revision.0, id.0, self.edit_session))
    }
    /// Return a bounded logical text window around the caret, anchor and optional pre-edit; a stale
    /// typing session publishes the document's current text, and unusable or non-boundary offsets
    /// return `Unavailable` rather than stale data.
    fn ime_surrounding(&self, cx: &ToolContext, max_bytes: usize) -> ImeSurrounding {
        let Some(id) = self.editing else {
            return ImeSurrounding::Unavailable("no text object is being edited");
        };
        let Some(current) = Self::text(cx, id) else {
            return ImeSurrounding::Unavailable("the edited text object is gone");
        };
        // The document changed under the typing session (undo, a panel or control edit): the
        // session's runs, marked-text range and caret offsets no longer describe the document.
        // Publish the document's own text (never the stale session's, which the IME cannot see);
        // the marked text has no range in it, so it is excluded from the logical view. The
        // commit-time check (`ime_session_matches_document`) reports the reset as application
        // origin instead of letting a receipt describe the wrong text.
        let stale = self.typing.as_ref().is_some_and(|ty| ty.cur != current.runs);
        if self.preedit.is_some() && self.typing.is_none() {
            return ImeSurrounding::Unavailable("the IME pre-edit has no typing session");
        }
        let Some(segments) = self.plain_segments(cx) else {
            return ImeSurrounding::Unavailable("the edited text object is gone");
        };
        let len = plain_len(&segments);
        let caret = self.caret.min(len);
        let anchor = self.anchor.min(len);
        if !plain_is_boundary(&segments, caret) || !plain_is_boundary(&segments, anchor) {
            return ImeSurrounding::Unavailable("the caret or selection anchor is not a UTF-8 boundary");
        }
        let preedit = match self.preedit.as_ref().filter(|_| !stale) {
            Some(p) => {
                let start = p.range.start.min(len);
                let end = p.range.end.min(len);
                if start > end || !plain_is_boundary(&segments, start) || !plain_is_boundary(&segments, end) {
                    return ImeSurrounding::Unavailable("the IME pre-edit is not on UTF-8 boundaries");
                }
                Some(start..end)
            }
            None => None,
        };
        ime_surrounding_window(&segments, preedit, len, caret, anchor, max_bytes)
    }
    /// Whether an IME pre-edit is currently shown for this Type-tool session.
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
        self.prev_tool = None;
        self.finish(cx)
    }
    /// Editing ends when a command takes the text out of the selection (Deselect, another object
    /// picked in the Layers panel) or removes it. After an Undo that takes back a list typed
    /// characters started, the caret goes on after the characters, and after its Redo the caret
    /// goes before the paragraph's text.
    fn after_command(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.after_typed_list(cx);
        self.after_outside_command(cx);
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
    /// After `text.create` / `text.createInPath` the engine selects the new object; edit it. After
    /// `edit.undo` / `edit.redo` the caret goes to where the step changed the edited text. The
    /// `batch.*` notices keep what a failed `command.batch` changed from staying ([`TypeTool::batch_notice`]).
    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if matches!(what, "edit.undo" | "edit.redo") {
            self.after_history_step(cx, what == "edit.undo");
        } else if what.starts_with("batch.") {
            self.batch_notice(what);
        } else if what == "text.editNew"
            && let Some(id) = cx.selection.objects.first().copied()
        {
            self.start_editing(id, 0);
            self.remember(cx);
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
                json!({
                    "editing": id.0,
                    "start": a,
                    "end": b,
                    "caret": self.caret,
                    "anchor": self.anchor,
                    "typing": self.typing.is_some(),
                    "composing": self.preedit.is_some(),
                    "prevTool": self.prev_tool,
                })
            }
            None => json!({"editing": null, "prevTool": self.prev_tool}),
        }
    }
    /// `select {start, end}` / `selectAll` / `copy` (with `runs` of the selection) /
    /// `commitTyping` (the caller commits the engine interaction).
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            // Transient selection transferred between Type and Eyedropper; never a saved setting.
            "textSelection" => {
                if let (Some(id), Some(a), Some(b)) = (
                    v.get("editing").and_then(Value::as_u64),
                    v.get("anchor").or_else(|| v.get("start")).and_then(Value::as_u64),
                    v.get("caret").or_else(|| v.get("end")).and_then(Value::as_u64),
                ) && let (Ok(a), Ok(b)) = (usize::try_from(a), usize::try_from(b))
                {
                    self.start_editing(NodeId(id), b);
                    self.anchor = a;
                }
            }
            "prevTool" | "previousTool" => {
                self.prev_tool = v.as_str().map(String::from);
            }
            "commitTyping" | "endTyping" => {
                self.typing = None;
                self.preedit = None;
                self.typing_ended = true;
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
                self.prev_tool = None;
            }
            _ => {}
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let (Some((_, s)), Some(d)) = (self.press, self.drag) {
            // The area being drawn (along the Constrain Angle's axes when one is set, as `on_up`
            // makes it) and its size.
            let size = match vectorcraft_geom::constrain_axes(cx.constrain_angle) {
                None => {
                    let r = Rect::from_points(s, d);
                    o.push(Overlay::Marquee(r));
                    r.size()
                }
                Some(axes) => {
                    let back = axes.inverse();
                    let r = Rect::from_points(back * s, back * d);
                    let mut path = r.to_path(0.1);
                    path.apply_affine(axes);
                    o.push(Overlay::Path { path, color: [79, 128, 255], width: 1.0, dashed: true });
                    r.size()
                }
            };
            if cx.measurement_labels {
                o.push(Overlay::Measure { p: d, text: cx.size_label(size.width, size.height) });
            }
        }
        // The ports of selected area type and type on a path, as many as show ([`PORTS_MAX`]), the
        // edited one's included.
        let shown = ports_shown(cx.doc, &cx.selection.objects);
        if shown {
            for id in cx.selection.objects.iter().filter(|id| Some(**id) != self.editing) {
                if let Some(t) = ported(cx.doc, *id) {
                    let lay = self.layout(t);
                    ports(cx, *id, t, lay.overflow, &mut o);
                }
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
        if let Some(id) = self.editing.filter(|id| shown && ported(cx.doc, *id).is_some()) {
            ports(cx, id, &t, lay.overflow, &mut o);
        }
        o
    }
    /// The pointer of the Type tool that a press would act as with the keys held, or, while a
    /// press is held, of the one it acts as ([`TypeTool::acting`]).
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        let (mode, vertical) = match (self.press, self.held) {
            (Some(_), Some(held)) => held,
            _ => self.acting(m),
        };
        if mode != Mode::Type && Self::path_at(cx, p, mode == Mode::Area).is_some() {
            return Cursor::Crosshair;
        }
        if vertical { Cursor::TypeVertical } else { Cursor::Text }
    }
}
