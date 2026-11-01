use super::{
    CommandSpec, EngineError, NodeId, Result, SELECTS_GIVEN_IDS, Selection, Session, Value, cmd, find_command, given_ids, guard, strip_replay_only,
};

impl Session {
    /// `spec`'s `enabled` with `ids` selected instead of the selection, which is then put back.
    fn enabled_with_selection(&mut self, spec: &CommandSpec, ids: Vec<NodeId>) -> std::result::Result<(), String> {
        let Some(d) = self.active_mut() else { return (spec.enabled)(self) };
        let old = std::mem::replace(&mut d.selection, Selection { objects: ids, ..Selection::default() });
        let r = (spec.enabled)(self);
        if let Some(d) = self.active_mut() {
            d.selection = old;
        }
        r
    }

    /// Error, under the command's own id, at the first of `ids` that is malformed, names no node, or
    /// names a layer or sublayer: a command in [`SELECTS_GIVEN_IDS`] acts on objects.
    fn check_given_ids(&self, id: &str, params: &Value) -> Result<()> {
        let (Some(st), Some(ids)) = (self.active(), params.get("ids")) else { return Ok(()) };
        let Some(given) = ids.as_array() else { return cmd::checked_ids_param(self, params, "ids", id).map(drop) };
        for value in given {
            let node = cmd::checked_id(self, value, id)?;
            if st.doc.node(node).is_some_and(|n| n.is_layer()) {
                return Err(if id.starts_with("object.arrange.") {
                    cmd::bad(id, format!("id {value} is a layer, not an object; move layers with layer.move"))
                } else {
                    cmd::bad(id, format!("id {value} is a layer, not an object"))
                });
            }
        }
        Ok(())
    }

    /// Execute a command by id. This is THE entry point for every frontend.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| cmd::params::unknown_command(id))?;
        // The crate's own tests are the audit of the params descriptions (#1442): a key a test
        // passes that its command's description doesn't name fails the test.
        #[cfg(test)]
        self.check_params(id, params)?;
        // Objects given as `id` (where it stands for `ids` of one) or as `ids` beside it are checked
        // before anything else: a malformed one, or one naming no object, is an error under this
        // command's id, never the selection in its place. `effect.apply {id: "stylize.dropShadow"}`
        // without `effect` names the effect (its older key), not an object.
        let effect_named = id == "effect.apply"
            && params.get("effect").is_none()
            && params.get("ids").is_none_or(Value::is_null)
            && params.get("id").and_then(Value::as_str).is_some_and(|e| !e.starts_with('$'));
        if cmd::params::id_stands_for_ids(spec.params) && !effect_named && self.active().is_some() {
            cmd::given_objects(self, params, id)?;
        }
        if SELECTS_GIVEN_IDS.contains(&id) {
            self.check_given_ids(id, params)?;
        }
        // Clear's menu needs a selection; an agent supplying explicit targets only needs a
        // document. The command validates those targets before making any edit.
        let given = given_ids(spec.params, params);
        let enabled = if id == "edit.clear" && params.get("ids").is_some() {
            cmd::has_doc(self)
        } else if let Some(ids) = given.clone() {
            // Objects given as `ids` (or `id`) stand in for the selection: the command is enabled
            // as it is with them selected (#1575).
            self.enabled_with_selection(spec, ids)
        } else {
            (spec.enabled)(self)
        };
        if let Err(why) = enabled {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        // Only top-level commands are journaled (commands that call other commands would otherwise
        // be recorded twice and replay differently).
        if self.depth == 0 {
            self.journal_note.clear();
            self.note_depth = 1;
            self.journal_query = false;
            self.batch_stash = None;
            self.batch_steps = None;
        }
        // A selection-only command given `ids` (or `id`) selects them first, in the same undo step
        // (#2130); when the command then fails, the selection (and the current layer selecting
        // them made current) goes back to how it was, as when the ids are refused.
        let select_first = SELECTS_GIVEN_IDS
            .contains(&id)
            .then(|| {
                params.get("ids").filter(|v| !v.is_null()).cloned().or_else(|| given.map(|ids| ids.iter().map(|i| i.0).collect::<Vec<_>>().into()))
            })
            .flatten();
        let selected_before = select_first.as_ref().and(self.active()).map(|d| (d.uid, d.selection.clone(), d.active_layer));
        let run = |s: &mut Session| {
            if let Some(ids) = &select_first {
                s.execute("select.set", &serde_json::json!({ "ids": ids }))?;
            }
            let r = (spec.run)(s, params);
            if r.is_err()
                && let Some((uid, selection, layer)) = selected_before
                && let Some(st) = s.docs.iter_mut().find(|d| d.uid == uid)
            {
                st.selection = selection;
                st.active_layer = layer;
                st.revision += 1;
            }
            r
        };
        // The entries the command journals belong to the undo groups of the documents it edits, and to
        // the group of the document it starts in when it ends there too.
        let (doc, journal, grouped) = (self.active().map(|d| d.uid), self.journal.len(), self.undo_group_docs());
        let open = (self.depth == 0).then(|| self.docs.iter().map(|d| d.uid).collect::<Vec<_>>());
        let r = if self.depth == 0 { self.run_guarded(id, run) } else { self.run_nested(run) };
        let r = r?;
        if self.depth == 0 {
            self.inherit_new_art();
        }
        if let Some(open) = open {
            self.open_undo_groups_in_new_documents(&open);
        }
        let journaled = spec.journal || (self.depth == 0 && std::mem::take(&mut self.journal_query));
        if journaled && self.depth == 0 && self.active().is_none_or(|d| d.interaction.is_none()) {
            let p = self.noted(params);
            self.journal.push((id.to_string(), p));
        }
        if self.depth == 0 {
            self.journal_in_undo_groups(doc, &grouped, journal);
            self.after_command(Some(id));
        } else if matches!(id, "edit.undo" | "edit.redo") {
            // A step of `command.batch`: the tool hears of it as it does of the command alone.
            self.notify_text_tool(id);
        }
        Ok(r)
    }

    /// Is the interface in Japanese? Then new type starts with em box top-to-top leading and em
    /// box centre character alignment (#432).
    pub fn japanese_interface(&self) -> bool {
        self.ui_language.as_deref().unwrap_or(&self.prefs.interface_language).eq_ignore_ascii_case("ja")
    }

    /// Record `key: value` in the running top-level command's journal entry (or its interaction's
    /// preview, or its step in a batch) unless its params give `key`: a value it resolved from the
    /// preferences or the clock.
    pub fn note_journal(&mut self, key: &str, value: Value) {
        if self.depth == self.note_depth {
            self.journal_note.insert(key.to_string(), value);
        }
    }

    /// Journal the running query command when it is the top-level command: it changed the
    /// selection (`document.find {select: true}`), so an action recorded around it repeats that.
    pub(crate) fn journal_query(&mut self) {
        if self.depth == 1 {
            self.journal_query = true;
        }
    }

    /// Run `id` as a step of the running command, which journals its steps (`command.batch`):
    /// → the step's result and its params with what it noted ([`Session::note_journal`]), for the
    /// step in the running command's journal entry.
    pub(crate) fn execute_step(&mut self, id: &str, params: &Value) -> Result<(Value, Value)> {
        let outer = (std::mem::take(&mut self.journal_note), self.note_depth);
        self.note_depth = self.depth + 1;
        let r = self.execute(id, params);
        let noted = self.noted(params);
        (self.journal_note, self.note_depth) = outer;
        Ok((r?, noted))
    }

    /// The journal from entry `start` on, as an action records it: without the params that only
    /// pin a replay to the original run ([`REPLAY_ONLY`]), so playing the action later acts now.
    pub fn journal_for_action(&self, start: usize) -> Vec<(String, Value)> {
        self.journal
            .since(start)
            .iter()
            .cloned()
            .map(|(id, mut p)| {
                strip_replay_only(&id, &mut p);
                (id, p)
            })
            .collect()
    }

    /// `params` with the noted values added (see [`Session::note_journal`]).
    pub(super) fn noted(&mut self, params: &Value) -> Value {
        let mut note = std::mem::take(&mut self.journal_note);
        match params {
            // The params' own values win.
            Value::Object(m) if !note.is_empty() => {
                note.extend(m.clone());
                Value::Object(note)
            }
            Value::Null if !note.is_empty() => Value::Object(note),
            _ => params.clone(),
        }
    }

    pub(crate) fn run_nested(&mut self, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        self.depth += 1;
        let r = f(self);
        self.depth -= 1;
        r
    }

    /// A top-level command: a panic (a bug) becomes [`EngineError::Internal`] and every open
    /// document goes back to how it was before the command (its art, selection, interaction and
    /// undo history), and the same document stays active, instead of crashing the frontend.
    pub(crate) fn run_guarded(&mut self, id: &str, f: impl FnOnce(&mut Self) -> Result<Value>) -> Result<Value> {
        let snapshot: Vec<_> =
            self.docs.iter().map(|d| (d.uid, d.doc.clone(), d.selection.clone(), d.interaction.clone(), d.history.clone())).collect();
        let active = self.active().map(|d| d.uid);
        let r = match guard::catch_panic(|| self.run_nested(f)) {
            Ok(r) => r,
            Err(msg) => {
                self.depth = 0;
                self.journal_note.clear();
                for (uid, doc, selection, interaction, history) in snapshot {
                    // A document the command closed stays closed.
                    let Some(st) = self.docs.iter_mut().find(|d| d.uid == uid) else { continue };
                    st.doc = doc;
                    st.selection = selection;
                    st.interaction = interaction;
                    st.history = history;
                    cmd::globaledit::follow_history(st);
                    st.revision += 1;
                }
                if let Some(i) = active.and_then(|uid| self.docs.iter().position(|d| d.uid == uid)) {
                    self.active = Some(i);
                }
                Err(EngineError::Internal { cmd: id.to_string(), msg })
            }
        };
        // Undo, Redo and a failed batch bring back documents with their selections: each document
        // gets its selection's new art on a locked artboard.
        for st in &mut self.docs {
            st.sync_new_art();
        }
        r
    }
}
