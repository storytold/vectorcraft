use super::*;

/// One undo step.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub label: String,
    /// The document as it was. It keeps no lookup cache ([`Document::drop_lookup_cache`]): a
    /// table per step would add up, and undoing builds it again once.
    pub doc: Arc<Document>,
    /// The selection as it was, shared with the neighbouring steps that have the same one.
    pub selection: Arc<Selection>,
    /// Roughly the memory this step keeps of its own: what of `doc` the step after it (or the
    /// document now) doesn't share ([`Document::retained_bytes`]).
    pub bytes: usize,
    /// The same the other way round: what that next state keeps that `doc` doesn't share. Undoing
    /// (redoing) this step makes a step that keeps the next state: this is its `bytes`.
    next_bytes: usize,
    /// Which document the figures were taken against (its address): the next state.
    next: usize,
    /// The template and name `label` was made from ([`Session::edit_named`]).
    pub(crate) named: Option<NamedStep>,
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
        HistoryEntry { label, doc, selection, bytes, next_bytes, next: std::ptr::from_ref(next) as usize, named: None }
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
        HistoryEntry { label, doc, selection, bytes: costs.next_bytes, next_bytes: costs.bytes, next: Arc::as_ptr(now) as usize, named: None }
    }

    /// The figures [`HistoryEntry::turned`] reuses.
    pub(crate) fn costs(&self) -> StepCosts {
        StepCosts { bytes: self.bytes, next_bytes: self.next_bytes, next: self.next }
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
    /// steps go, but never the last [`History::MIN_STEPS`] undo (and redo) steps.
    pub byte_budget: usize,
}

impl Default for History {
    fn default() -> Self {
        History { undo: vec![], redo: vec![], limit: 0, byte_budget: default_history_budget() }
    }
}

impl History {
    /// Undo (and redo) steps kept whatever memory they take.
    pub const MIN_STEPS: usize = 8;

    /// Roughly the memory the steps keep of their own ([`HistoryEntry::bytes`]).
    pub fn bytes(&self) -> usize {
        self.undo.iter().chain(&self.redo).map(|e| e.bytes).sum()
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

    /// Estimate the newest undo step again, its next state now being `now` (the steps after it were
    /// merged into it, or an undo group extended it).
    pub(crate) fn recost_last(&mut self, now: &Document) {
        if let Some(e) = self.undo.last_mut() {
            (e.bytes, e.next_bytes) = e.doc.retained_bytes_each(now);
            e.next = std::ptr::from_ref(now) as usize;
        }
    }
}

/// The default [`History::byte_budget`]: an eighth of the machine's memory where it is known
/// (Linux), at most 1 GiB (the figure elsewhere); 256 MiB on the web, whose memory is small.
pub fn default_history_budget() -> usize {
    const MAX: usize = 1 << 30;
    if cfg!(target_arch = "wasm32") {
        return 256 << 20;
    }
    static BUDGET: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *BUDGET.get_or_init(|| physical_memory().map_or(MAX, |m| (m / 8).clamp(64 << 20, MAX)))
}

/// The machine's memory, where cheap to learn without platform calls.
fn physical_memory() -> Option<usize> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: usize = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}
