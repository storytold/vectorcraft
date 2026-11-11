use super::{Arc, Document, Selection};

#[cfg(test)]
thread_local! {
    /// Tests: how many undo steps were copied on this thread.
    pub(crate) static ENTRY_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One undo step.
#[derive(Debug)]
pub struct HistoryEntry {
    pub label: String,
    /// The document as it was. It keeps no lookup cache ([`Document::drop_lookup_cache`]): a
    /// table per step would add up, and undoing builds it again once.
    pub doc: Arc<Document>,
    /// The selection as it was, shared with the neighbouring steps that have the same one.
    pub selection: Arc<Selection>,
    /// Roughly the memory this step keeps of its own: the unshared document data
    /// ([`Document::retained_bytes`]) plus owned color-mode paint snapshots.
    pub bytes: usize,
    /// The same the other way round: what that next state keeps that `doc` doesn't share. Undoing
    /// (redoing) this step makes a step that keeps the next state: this is its `bytes`.
    next_bytes: usize,
    /// Which document the figures were taken against (its address): the next state.
    next: usize,
    /// The template and name `label` was made from ([`Session::edit_named`]).
    pub(crate) named: Option<NamedStep>,
    /// A number no other step takes ([`History::step`]).
    serial: u64,
    /// Original and converted new-art colors, only for steps that change document color mode.
    pub(crate) color_mode_paints: Option<(vectorcraft_tools::PaintDefaults, vectorcraft_tools::PaintDefaults)>,
    /// Owned new-art snapshots included in both directional estimates.
    color_mode_bytes: usize,
}

impl Clone for HistoryEntry {
    fn clone(&self) -> Self {
        #[cfg(test)]
        ENTRY_CLONES.with(|n| n.set(n.get() + 1));
        HistoryEntry {
            label: self.label.clone(),
            doc: self.doc.clone(),
            selection: self.selection.clone(),
            bytes: self.bytes,
            next_bytes: self.next_bytes,
            next: self.next,
            named: self.named.clone(),
            serial: self.serial,
            color_mode_paints: self.color_mode_paints.clone(),
            color_mode_bytes: self.color_mode_bytes,
        }
    }
}

/// The next [`HistoryEntry::serial`]: 1 for the first step made, so 0 never names a step.
fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A step name made by filling the `{name}` placeholder of a template with a name
/// ([`Session::edit_named`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedStep {
    /// The English template, with one `{name}` placeholder (`Apply {name}`).
    pub template: &'static str,
    /// The name in the placeholder, as the document holds it (a style's name).
    pub name: String,
    /// The name is one VectorCraft gives (`[Normal Character Style]`), not one the user typed.
    pub built_in: bool,
}

impl NamedStep {
    /// The step name: the template with the name in its placeholder.
    pub fn label(&self) -> String {
        self.template.replace("{name}", &self.name)
    }
}

impl HistoryEntry {
    /// A step keeping `doc` and `selection`, as they were before the edit that made `next`.
    pub(crate) fn kept(label: String, doc: Arc<Document>, selection: Arc<Selection>, next: &Document) -> Self {
        doc.drop_lookup_cache();
        let (bytes, next_bytes) = doc.retained_bytes_each(next);
        HistoryEntry {
            label,
            doc,
            selection,
            bytes,
            next_bytes,
            next: std::ptr::from_ref(next) as usize,
            named: None,
            serial: next_serial(),
            color_mode_paints: None,
            color_mode_bytes: 0,
        }
    }

    /// The template and name the step's label was made from, while the label is still the one
    /// they make: `None` for a step named any other way, and for one whose label was changed
    /// since it was recorded.
    pub fn named(&self) -> Option<&NamedStep> {
        self.named.as_ref().filter(|n| n.label() == self.label)
    }

    /// The step that undoing (redoing) the step `costs` came from makes: it keeps `doc`, the
    /// document before the undo, whose next state is now `now` (that step's document). Its figures
    /// are that step's the other way round when `doc` is still the document they were taken
    /// against (nothing replaced it since), so undo and redo needn't walk the documents again. A
    /// change made to it in place without an undo step (a setting, a refresh) goes unnoticed: a
    /// small error, for an estimate.
    pub(crate) fn turned(label: String, doc: Arc<Document>, selection: Arc<Selection>, now: &Arc<Document>, costs: StepCosts) -> Self {
        if costs.next != Arc::as_ptr(&doc) as usize {
            return Self::kept(label, doc, selection, now);
        }
        doc.drop_lookup_cache();
        HistoryEntry {
            label,
            doc,
            selection,
            bytes: costs.next_bytes,
            next_bytes: costs.bytes,
            next: Arc::as_ptr(now) as usize,
            named: None,
            serial: next_serial(),
            color_mode_paints: None,
            color_mode_bytes: 0,
        }
    }

    /// Recount the separately owned paints after a color-mode step or group changes them.
    pub(crate) fn recost_paints(&mut self) {
        let owned = |p: &vectorcraft_tools::PaintDefaults| {
            std::mem::size_of::<vectorcraft_tools::PaintDefaults>()
                + vectorcraft_doc::paint_heap_bytes(&p.fill)
                + vectorcraft_doc::paint_heap_bytes(&p.stroke)
                + p.appearance.as_ref().map_or(0, vectorcraft_doc::appearance_heap_bytes)
                + p.style.as_ref().map_or(0, String::capacity)
        };
        let bytes = self.color_mode_paints.as_ref().map_or(0, |(before, after)| owned(before).saturating_add(owned(after)));
        self.bytes = self.bytes.saturating_sub(self.color_mode_bytes).saturating_add(bytes);
        self.next_bytes = self.next_bytes.saturating_sub(self.color_mode_bytes).saturating_add(bytes);
        self.color_mode_bytes = bytes;
    }

    /// The document-only figures [`HistoryEntry::turned`] reuses. Its paint snapshots are
    /// attached and counted separately, so swapping direction never doubles their cost.
    pub(crate) fn costs(&self) -> StepCosts {
        StepCosts {
            bytes: self.bytes.saturating_sub(self.color_mode_bytes),
            next_bytes: self.next_bytes.saturating_sub(self.color_mode_bytes),
            next: self.next,
        }
    }
}

/// A step's memory figures ([`HistoryEntry::costs`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepCosts {
    bytes: usize,
    next_bytes: usize,
    next: usize,
}

/// The undo history of a document: bounded by a number of steps (Preferences › Performance ›
/// History States) and by the memory the steps keep ([`History::byte_budget`]).
#[derive(Clone, Debug)]
pub struct History {
    pub undo: Vec<HistoryEntry>,
    pub redo: Vec<HistoryEntry>,
    /// Maximum undo depth (Illustrator has no hard limit; snapshots are cheap here).
    pub limit: usize,
    /// About the most memory the steps may keep ([`HistoryEntry::bytes`]): past it the oldest
    /// steps go, but never the last [`History::MIN_STEPS`] undo (and redo) steps. The histories of a
    /// session's open documents also share [`Session::history_budget`](crate::Session::history_budget).
    pub byte_budget: usize,
    /// Counts the changes made in place to steps below the newest ([`History::put_back`]).
    pub(crate) rewrites: u64,
    /// Between [`History::mark`] and [`History::release`] or [`History::put_back`].
    pub(crate) held: bool,
    /// The redo steps a new step cleared while held ([`History::clear_redo`]).
    pub(crate) set_aside: Option<Vec<HistoryEntry>>,
}

impl Default for History {
    fn default() -> Self {
        History { undo: vec![], redo: vec![], limit: 0, byte_budget: default_history_budget(), rewrites: 0, held: false, set_aside: None }
    }
}

/// What [`History::put_back`] needs to give a history back as it was when [`History::mark`] took
/// it: the newest undo and redo steps, and where each stack stood under them. It holds no other
/// step, so taking one costs the same however many the history holds.
pub(crate) struct HistoryMark {
    undo: StackMark,
    redo: StackMark,
    limit: usize,
    byte_budget: usize,
    rewrites: u64,
}

/// One stack of a [`HistoryMark`].
struct StackMark {
    len: usize,
    /// [`HistoryEntry::serial`] of the step under the newest one, 0 for none.
    below: u64,
    newest: Option<HistoryEntry>,
}

impl StackMark {
    fn of(steps: &[HistoryEntry]) -> Self {
        let len = steps.len();
        StackMark { len, below: len.checked_sub(2).and_then(|i| steps.get(i)).map_or(0, |e| e.serial), newest: steps.last().cloned() }
    }

    /// Puts `steps` back as they were. → `false` when a step under the newest one was removed since.
    fn put_back(self, steps: &mut Vec<HistoryEntry>) -> bool {
        let kept = self.len.saturating_sub(1);
        let intact = steps.len() >= kept && kept.checked_sub(1).is_none_or(|i| steps.get(i).is_some_and(|e| e.serial == self.below));
        steps.truncate(kept);
        steps.extend(self.newest);
        intact
    }
}

impl History {
    /// Undo (and redo) steps kept whatever memory they take.
    pub const MIN_STEPS: usize = 8;

    /// Roughly the memory the steps keep of their own ([`HistoryEntry::bytes`]).
    pub fn bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|e| e.bytes).sum()
    }

    /// Which step the document is at: the newest undo step and the next redo step, by numbers no
    /// other step takes (0 for none). Every new step, Undo, Redo and edit merged into the newest
    /// step (an undo group) changes it; a selection change does not.
    pub fn step(&self) -> (u64, u64) {
        (self.undo.last().map_or(0, |e| e.serial), self.redo.last().map_or(0, |e| e.serial))
    }

    /// The document at row `row` of the History panel, `now` being the document now: the rows before
    /// it are the undo steps, oldest first, and the rows after it the redo steps, nearest first.
    pub(crate) fn state<'a>(&'a self, now: &'a Arc<Document>, row: usize) -> Option<&'a Arc<Document>> {
        let current = self.undo.len();
        match row.cmp(&current) {
            std::cmp::Ordering::Less => self.undo.get(row).map(|e| &e.doc),
            std::cmp::Ordering::Equal => Some(now),
            std::cmp::Ordering::Greater => self.redo.len().checked_sub(row - current).and_then(|i| self.redo.get(i)).map(|e| &e.doc),
        }
    }

    /// Let the oldest steps go while over the step limit or the memory budget: undo steps first
    /// (the oldest), then redo steps (the furthest ahead).
    pub fn trim(&mut self) {
        let over = self.undo.len().saturating_sub(self.limit);
        if over > 0 {
            self.undo.drain(..over);
        }
        let mut total = self.bytes();
        if total <= self.byte_budget {
            return;
        }
        let mut n = 0;
        while total > self.byte_budget && self.undo.len() - n > Self::MIN_STEPS {
            total -= self.undo.get(n).map_or(0, |e| e.bytes);
            n += 1;
        }
        self.undo.drain(..n);
        let mut n = 0;
        while total > self.byte_budget && self.redo.len() - n > Self::MIN_STEPS {
            total -= self.redo.get(n).map_or(0, |e| e.bytes);
            n += 1;
        }
        self.redo.drain(..n);
    }

    /// A [`HistoryMark`] of the history now. Until [`History::release`] or [`History::put_back`],
    /// [`History::trim`] lets no step go, so the steps under the newest one stay where they are, and
    /// [`History::clear_redo`] sets the redo steps aside.
    pub(crate) fn mark(&mut self) -> HistoryMark {
        let mark = HistoryMark {
            undo: StackMark::of(&self.undo),
            redo: StackMark::of(&self.redo),
            limit: self.limit,
            byte_budget: self.byte_budget,
            rewrites: self.rewrites,
        };
        self.limit = usize::MAX;
        self.byte_budget = usize::MAX;
        self.held = true;
        self.set_aside = None;
        mark
    }

    /// Removes the redo steps, as a new step does. While held ([`History::mark`]) the first call
    /// keeps them for [`History::put_back`].
    pub(crate) fn clear_redo(&mut self) {
        if self.held && self.set_aside.is_none() {
            self.set_aside = Some(std::mem::take(&mut self.redo));
        } else {
            self.redo.clear();
        }
    }

    /// Keeps the history as it is now: the limits `mark` held back apply again.
    pub(crate) fn release(&mut self, mark: &HistoryMark) {
        self.held = false;
        self.set_aside = None;
        self.limit = mark.limit;
        self.byte_budget = mark.byte_budget;
        self.trim();
    }

    /// Gives the history back as it was at `mark`. → `false` when that is not possible: an undo or
    /// redo step under the newest one was removed or changed in place since.
    pub(crate) fn put_back(&mut self, mark: HistoryMark) -> bool {
        if let Some(redo) = self.set_aside.take() {
            self.redo = redo;
        }
        let undo = mark.undo.put_back(&mut self.undo);
        let redo = mark.redo.put_back(&mut self.redo);
        let intact = self.rewrites == mark.rewrites && undo && redo;
        self.held = false;
        self.limit = mark.limit;
        self.byte_budget = mark.byte_budget;
        intact
    }

    /// Estimate the newest undo step again, its next state now being `now` (the steps after it were
    /// merged into it, or an undo group extended it). It takes a new number ([`History::step`]).
    pub(crate) fn recost_last(&mut self, now: &Document) {
        if let Some(e) = self.undo.last_mut() {
            let (before, after) = e.doc.retained_bytes_each(now);
            e.bytes = before.saturating_add(e.color_mode_bytes);
            e.next_bytes = after.saturating_add(e.color_mode_bytes);
            e.next = std::ptr::from_ref(now) as usize;
            e.serial = next_serial();
        }
    }
}

/// Let the oldest steps of `histories` go while together they keep more than `budget`
/// ([`History::bytes`]): undo steps first, the oldest of all the histories first, then redo steps,
/// the furthest ahead first. Each history keeps its last [`History::MIN_STEPS`] undo (and redo)
/// steps, and a held one ([`History::mark`]) keeps all of its steps; their memory still counts.
pub(crate) fn trim_together<'a>(histories: impl IntoIterator<Item = &'a mut History>, budget: usize) {
    let all: Vec<&mut History> = histories.into_iter().collect();
    let mut total = all.iter().fold(0usize, |t, h| t.saturating_add(h.bytes()));
    if total <= budget {
        return;
    }
    fn steps(h: &History, redo: bool) -> &[HistoryEntry] {
        if redo { &h.redo } else { &h.undo }
    }
    let mut free: Vec<&mut History> = all.into_iter().filter(|h| !h.held).collect();
    for redo in [false, true] {
        // How many of each history's oldest steps go.
        let mut gone = vec![0usize; free.len()];
        while total > budget {
            // The oldest step any history may still let go.
            let next = free
                .iter()
                .zip(&gone)
                .enumerate()
                .filter(|(_, (h, n))| steps(h, redo).len().saturating_sub(**n) > History::MIN_STEPS)
                .filter_map(|(i, (h, n))| steps(h, redo).get(*n).map(|e| (e.serial, i, e.bytes)))
                .min();
            let Some((_, i, bytes)) = next else { break };
            total = total.saturating_sub(bytes);
            if let Some(n) = gone.get_mut(i) {
                *n += 1;
            }
        }
        for (h, n) in free.iter_mut().zip(gone) {
            let steps = if redo { &mut h.redo } else { &mut h.undo };
            steps.drain(..n.min(steps.len()));
        }
    }
}

/// What [`Session::trim_histories`](crate::Session::trim_histories) last left: the budget, and for
/// each open document its uid and what a new, undone, redone, merged, recosted, removed or put-back
/// step changes in its history.
pub(crate) type HistoriesSeen = (usize, Vec<(u64, (u64, u64), usize, usize, usize, u64, bool)>);

impl crate::Session {
    /// Let the oldest undo and redo steps of the open documents go while together they keep more
    /// than [`Session::history_budget`](crate::Session::history_budget) ([`trim_together`]). While a
    /// script runs, its commands leave the histories as they are: the run's end trims them. While an
    /// undo group is open in any document, the histories stay as they are too, so a canceled group
    /// leaves the other documents' steps in place: closing the groups trims them. When
    /// neither the budget nor any history changed since the last call, the steps are not counted
    /// again.
    pub(crate) fn trim_histories(&mut self) {
        if self.script_in_run || self.docs.iter().any(|d| d.undo_group.is_some()) {
            return;
        }
        if self.histories_seen.as_ref() == Some(&self.histories_now()) {
            return;
        }
        trim_together(self.docs.iter_mut().map(|d| &mut d.history), self.history_budget);
        self.histories_seen = Some(self.histories_now());
    }

    fn histories_now(&self) -> HistoriesSeen {
        let doc = |d: &crate::DocState| {
            let h = &d.history;
            (d.uid, h.step(), h.undo.len(), h.redo.len(), h.undo.last().map_or(0, |e| e.bytes), h.rewrites, h.held)
        };
        (self.history_budget, self.docs.iter().map(doc).collect())
    }
}

/// The default [`History::byte_budget`] and [`Session::history_budget`](crate::Session::history_budget),
/// which the open documents' histories share: a quarter of the installed memory, or on Linux of the
/// process's cgroup memory limit where that is lower ([`vectorcraft_render::physical_memory`], see
/// [`history_budget_for`]); 1 GiB where the installed memory can't be read, and 256 MiB on the web.
pub fn default_history_budget() -> usize {
    if cfg!(target_arch = "wasm32") {
        return 256 << 20;
    }
    static BUDGET: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| history_budget_for(vectorcraft_render::physical_memory()))
}

/// The default [`History::byte_budget`] for a reading of `memory` bytes installed: a quarter of it,
/// at least 64 MiB, and at most a quarter of the address space (on 32-bit builds); 1 GiB when the
/// reading is `None` or 0.
pub(crate) fn history_budget_for(memory: Option<u64>) -> usize {
    memory.filter(|&m| m > 0).map_or(1 << 30, |m| usize::try_from(m / 4).unwrap_or(usize::MAX).clamp(64 << 20, usize::MAX / 4))
}
