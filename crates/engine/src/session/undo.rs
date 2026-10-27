use super::*;

impl Session {
    // ---------- undo groups (scrubbed numeric fields) ----------

    /// Open an undo group in the active document: until [`Session::end_undo_group`], the edits
    /// made there (commands, committed interactions) are one undo step. A scrubbed numeric field
    /// applies each value it passes as its own command, as a typed value is applied; the drag is
    /// one step.
    pub fn begin_undo_group(&mut self) {
        self.begin_named_undo_group(None);
    }

    /// [`Session::begin_undo_group`] with the undo step named `label` (else after its first edit).
    pub fn begin_named_undo_group(&mut self, label: Option<String>) {
        if let Some(st) = self.active_mut()
            && st.undo_group.is_none()
        {
            st.undo_group = Some(UndoGroup { first: None, journal: vec![], steps: vec![], label });
        }
    }

    /// Between `edit.undoGroup.begin` and `edit.undoGroup.end`, open an undo group with the begun
    /// group's name in each document that the top-level command just run opened or created (its
    /// [`DocState::uid`] is not in `open`). The group takes the edits after that command: the
    /// command's own edits there stay its undo step, and a canceled group keeps them, as the journal
    /// entry that made the document stays.
    pub(super) fn open_undo_groups_in_new_documents(&mut self, open: &[u64]) {
        let Some(label) = &self.explicit_undo_group else { return };
        for st in &mut self.docs {
            if !open.contains(&st.uid) && st.undo_group.is_none() {
                st.undo_group = Some(UndoGroup { first: None, journal: vec![], steps: vec![], label: label.clone() });
            }
        }
    }

    /// The documents with an undo group open ([`DocState::uid`]), each with the document it holds.
    pub(super) fn undo_group_docs(&self) -> Vec<(u64, *const Document)> {
        self.docs.iter().filter(|st| st.undo_group.is_some()).map(|st| (st.uid, Arc::as_ptr(&st.doc))).collect()
    }

    /// Note the journal entries from number `from` on in the undo groups they belong to: the group of
    /// each document in `grouped` ([`Session::undo_group_docs`] before the entries) that they changed,
    /// each group they opened and edited, and the group of the document with [`DocState::uid`] `doc`
    /// when they started in it and it is still the active one. A cancel drops them. The last entry of a command that changed the
    /// active document keeps the index of the document it left active. A `command.batch` entry also
    /// keeps, in each group, the steps that belong to it ([`Session::batch_steps`]).
    pub(super) fn journal_in_undo_groups(&mut self, doc: Option<u64>, grouped: &[(u64, *const Document)], from: usize) {
        let to = self.journal.len();
        let active = self.active().map(|d| d.uid);
        let switched = if active == doc { None } else { self.active };
        let batch = self.batch_steps.take().filter(|_| to > from && self.journal.get(to - 1).is_some_and(|(id, _)| id == "command.batch"));
        for st in &mut self.docs {
            let Some(g) = &mut st.undo_group else { continue };
            let started = doc == Some(st.uid) && active == doc;
            // A group the entries opened has an edit of theirs once it has a first edit.
            let edited = match grouped.iter().find(|&&(uid, _)| uid == st.uid) {
                Some(&(_, d)) => !std::ptr::eq(d, Arc::as_ptr(&st.doc)),
                None => g.first.is_some(),
            };
            if started || edited {
                // A committed interaction inside the command noted its entry already.
                let noted = g.journal.partition_point(|&(n, _)| n < from);
                g.journal.truncate(noted);
                g.journal.extend((from..to).map(|n| (n, switched.filter(|_| n + 1 == to))));
                g.steps.retain(|&(n, _)| n < from);
                if let Some(steps) = &batch {
                    let own = steps.iter().enumerate().filter(|(_, (uids, _))| uids.contains(&st.uid));
                    g.steps.push((to - 1, own.map(|(i, &(_, switched))| (i, switched)).collect()));
                }
            }
        }
    }

    /// Close the open undo groups: their edits stay one undo step or, `cancel`led (Escape), are
    /// undone and dropped from the journal. A group begun with `edit.undoGroup.begin` ends too, so
    /// documents opened after this open no undo group.
    pub fn end_undo_group(&mut self, cancel: bool) {
        self.explicit_undo_group = None;
        self.close_undo_groups(cancel, None);
    }

    /// [`Session::end_undo_group`] without ending a group begun with `edit.undoGroup.begin`, for the
    /// document with [`DocState::uid`] `only` alone when given. The groups that stay open follow the
    /// journal entries and batch steps of theirs that a cancel renumbers.
    fn close_undo_groups(&mut self, cancel: bool, only: Option<u64>) {
        let mut journal: Vec<(usize, Option<usize>)> = vec![];
        let mut steps: Vec<BatchSteps> = vec![];
        for st in &mut self.docs {
            if only.is_some_and(|uid| uid != st.uid) {
                continue;
            }
            let Some(g) = st.undo_group.take() else { continue };
            // Only while the group's step is still the newest one.
            if cancel
                && let Some(first) = g.first
                && st.history.undo.last().is_some_and(|e| Arc::ptr_eq(&e.doc, &first))
                && let Some(e) = st.history.undo.pop()
            {
                st.doc = e.doc;
                st.selection = Arc::unwrap_or_clone(e.selection);
                deselect_hidden(&self.menu, &st.doc, &mut st.selection);
                st.sync_new_art();
                cmd::globaledit::follow_history(st);
                st.revision += 1;
                // The journal drops only the canceled groups' own entries and keeps the ones made in other documents meanwhile.
                journal.extend(g.journal);
                steps.extend(g.steps);
            }
        }
        // A batch that edited two of the documents is in both groups.
        journal.sort_unstable();
        journal.dedup_by_key(|&mut (n, _)| n);
        // A batch with steps outside the canceled groups keeps those steps.
        let mut rewritten: Vec<(usize, Vec<Option<usize>>)> = vec![];
        journal.retain(|&(n, _)| match self.keep_batch_steps(n, &steps) {
            Some(kept) => {
                rewritten.push((n, kept));
                false
            }
            None => true,
        });
        // An entry that changed the active document becomes that change, so the entries after it run
        // in the same document when the journal is replayed.
        for &(n, index) in &journal {
            if let Some(index) = index {
                self.journal.replace(n, ("document.activate".to_string(), serde_json::json!({ "index": index })));
            }
        }
        let gone: Vec<usize> = journal.into_iter().filter(|(_, index)| index.is_none()).map(|(n, _)| n).collect();
        self.journal.remove(&gone);
        let below = |n: usize| gone.partition_point(|&m| m < n);
        for g in self.docs.iter_mut().filter_map(|st| st.undo_group.as_mut()) {
            g.journal.retain(|(n, _)| gone.binary_search(n).is_err());
            for (n, _) in &mut g.journal {
                *n -= below(*n);
            }
            g.steps.retain(|(n, _)| gone.binary_search(n).is_err());
            for (n, own) in &mut g.steps {
                if let Some((_, kept)) = rewritten.iter().find(|(r, _)| r == n) {
                    own.retain_mut(|(i, _)| kept.get(*i).copied().flatten().map(|k| *i = k).is_some());
                }
                *n -= below(*n);
            }
        }
    }

    /// When journal entry `n` is a `command.batch` with `steps` noted in canceled undo groups and
    /// other steps that are not `document.activate`: rewrite its `commands` without the noted steps
    /// (a noted step that changed the active document becomes that change) → each step's number in
    /// the rewritten `commands`, `None` for a noted step. `None` leaves the entry to be dropped as a
    /// whole.
    fn keep_batch_steps(&mut self, n: usize, steps: &[BatchSteps]) -> Option<Vec<Option<usize>>> {
        let noted: Vec<(usize, Option<usize>)> = steps.iter().filter(|(e, _)| *e == n).flat_map(|(_, s)| s.iter().copied()).collect();
        let (id, entry) = self.journal.get(n).filter(|(id, _)| id == "command.batch" && steps.iter().any(|(e, _)| *e == n))?;
        let commands = entry.get("commands").and_then(Value::as_array)?;
        let kept = |i: usize| !noted.iter().any(|&(s, _)| s == i);
        let activate = |c: &Value| c.get("command").and_then(Value::as_str) == Some("document.activate");
        if commands.iter().enumerate().all(|(i, c)| !kept(i) || activate(c)) {
            return None;
        }
        let (mut rewritten, mut numbers) = (Vec::with_capacity(commands.len()), Vec::with_capacity(commands.len()));
        for (i, c) in commands.iter().enumerate() {
            if kept(i) {
                numbers.push(Some(rewritten.len()));
                rewritten.push(c.clone());
            } else {
                numbers.push(None);
                if let Some(index) = noted.iter().find(|&&(s, _)| s == i).and_then(|&(_, index)| index) {
                    rewritten.push(serde_json::json!({ "command": "document.activate", "params": { "index": index } }));
                }
            }
        }
        let (id, mut entry) = (id.clone(), entry.clone());
        if let Some(o) = entry.as_object_mut() {
            o.insert("commands".into(), Value::Array(rewritten));
        }
        self.journal.replace(n, (id, entry));
        Some(numbers)
    }

    /// [`Session::end_undo_group`] for the open document with [`DocState::uid`] `uid` only (it may
    /// have closed since): its open undo group closes, and undo groups open in other documents stay
    /// open.
    fn end_document_undo_group(&mut self, uid: u64, cancel: bool) {
        self.close_undo_groups(cancel, Some(uid));
    }

    /// Make the edits that follow in the active document one undo step, until
    /// [`Session::end_undo_scope`]: open an undo group there or, when one is open already, join it,
    /// so that the edits are part of its step. `None` without a document.
    pub fn begin_undo_scope(&mut self) -> Option<UndoScope> {
        let journal = self.journal.len();
        let st = self.active()?;
        let uid = st.uid;
        let joined = st.undo_group.as_ref().map(|g| (st.doc.clone(), st.selection.clone(), g.first.clone()));
        if joined.is_none() {
            self.begin_undo_group();
        }
        Some(UndoScope { uid, journal, joined })
    }

    /// End `scope` in its own document, whichever document is active now; undo groups open in other
    /// documents stay open. The undo group the scope opened closes as [`Session::end_undo_group`]
    /// closes it. A scope that joined an open group leaves the group open; with `cancel`, it puts the
    /// document and its selection back as they were when the scope began and cuts the journal back to
    /// where the scope began (only while the group's step is still the newest one), and an undo step
    /// that the scope's first edit recorded goes too. The open undo groups drop the cut entries.
    pub fn end_undo_scope(&mut self, scope: UndoScope, cancel: bool) {
        let Some((doc, selection, first)) = scope.joined else {
            self.end_document_undo_group(scope.uid, cancel);
            return;
        };
        let menu = self.menu.clone();
        let Some(st) = self.document_mut(scope.uid) else { return };
        let Some(g) = &mut st.undo_group else { return };
        let newest = g.first.as_ref().is_some_and(|f| st.history.undo.last().is_some_and(|e| Arc::ptr_eq(&e.doc, f)));
        if !cancel || !newest {
            return;
        }
        match &first {
            // The scope's first edit recorded the group's step.
            None => {
                st.history.undo.pop();
                g.first = None;
            }
            Some(f) if g.first.as_ref().is_some_and(|now| Arc::ptr_eq(now, f)) => {}
            Some(_) => return,
        }
        st.doc = doc;
        st.selection = selection;
        cmd::globaledit::follow_history(st);
        deselect_hidden(&menu, &st.doc, &mut st.selection);
        st.history.recost_last(&st.doc);
        st.revision += 1;
        self.journal.truncate(scope.journal);
        // The next commands take the cut entries' numbers, so no open undo group keeps them.
        for g in self.docs.iter_mut().filter_map(|st| st.undo_group.as_mut()) {
            g.journal.retain(|&(n, _)| n < scope.journal);
            g.steps.retain(|&(n, _)| n < scope.journal);
        }
    }
}
