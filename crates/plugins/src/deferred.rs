//! Live-effect plug-ins while a document opens: the plug-in effects drawn for its first view share
//! a time budget of [`OPENING_BUDGET_MS`], and a stage that is not run for want of time, or that
//! gives no result, is deferred: it draws its input art and is listed with the reason
//! ([`Deferrals::list`]) until it runs again.
//!
//! One [`Deferrals`] belongs to each open document. Drawing that document on screen runs inside
//! [`Deferrals::view`], which applies its budget and its deferred stages to the plug-in effects
//! evaluated on that thread ([`crate::effect`]):
//!
//! - until [`Deferrals::first_view_done`], every run takes the lesser of its per-call limit and
//!   what is left of the budget, no run starts once the budget is spent, and a result that comes
//!   back after the budget ran out is not used;
//! - afterwards a deferred stage is not run again while its input and parameters stay the same; a
//!   stage whose input or parameters changed runs under the per-call limit;
//! - [`Deferrals::refresh`] runs deferred stages again with a fresh budget.
//!
//! Work outside a view (exports, commands, drawing without a screen) runs every stage under the
//! per-call limit. When such a run gives a result for a stage some document has deferred,
//! [`revision`] changes, so that drawings made with the deferred stage are drawn again.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The time the plug-in effects drawn for a document's first view share, in milliseconds.
pub const OPENING_BUDGET_MS: u64 = 2_000;

/// Why a stage is deferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// It was not run, or its result came too late, because the budget was spent.
    Budget,
    /// It ran to its per-call time limit.
    Timeout,
    /// Its plug-in isn't installed as a live effect.
    Missing,
    /// It trapped, panicked, returned art that was refused, or its parameters were refused.
    Failed,
    /// Safe Mode is on: no plug-in runs.
    SafeMode,
}

impl Reason {
    /// The reason as `effect.deferredStatus` names it.
    pub fn name(self) -> &'static str {
        match self {
            Reason::Budget => "budget",
            Reason::Timeout => "timeout",
            Reason::Missing => "missing",
            Reason::Failed => "failed",
            Reason::SafeMode => "safeMode",
        }
    }
}

/// Where a plug-in effect sits: the object, the index of the effect in its list, and the fill or
/// stroke whose list that is (`None`: the object's own effects).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Stage {
    pub object: u64,
    pub effect: usize,
    pub item: Option<usize>,
}

/// A deferred stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deferred {
    pub stage: Stage,
    /// The plug-in's id.
    pub plugin: String,
    pub reason: Reason,
    /// The input and parameters the stage was deferred with (the effect cache's key), 0 when the
    /// plug-in was not looked at (Safe Mode, a missing plug-in).
    pub(crate) key: u64,
}

/// What [`Deferrals::refresh`] did with the stages it ran again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// Stages that gave a result.
    pub refreshed: usize,
    /// Stages left deferred for want of time, or with input the refresh did not run.
    pub still_deferred: usize,
    /// Stages that ran out of their per-call time, failed or had no plug-in again.
    pub failed: usize,
}

/// Milliseconds on a monotonic clock, `None` from a clock that cannot be read.
pub type Clock = Arc<dyn Fn() -> Option<u64> + Send + Sync>;

/// Milliseconds since the first reading in this process, from `web_time::Instant`: the system's
/// monotonic clock on the desktop and the page's `performance.now()` in the web build.
fn monotonic() -> Option<u64> {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(web_time::Instant::now);
    Some(u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Runs share a budget that started at `started` (`None`: no run yet). `first`: the first
    /// view, which reports what it deferred when it ends.
    Budgeted { started: Option<u64>, first: bool },
    /// Deferred stages are left as they are while their input stays the same.
    Open,
}

/// The deferred records in the order they were made, indexed by stage and cache key. A stage holds
/// at most one record for each key.
#[derive(Default)]
struct Records {
    /// The order number of the next record.
    next: u64,
    by_order: BTreeMap<u64, Deferred>,
    /// For each stage with records, the order number of its record for each key.
    by_stage: HashMap<Stage, HashMap<u64, u64>>,
}

impl Records {
    /// Every record, oldest first.
    fn iter(&self) -> impl Iterator<Item = &Deferred> {
        self.by_order.values().inspect(|_| looked_at(1))
    }

    /// The stages with records, in no particular order.
    fn stages(&self) -> impl Iterator<Item = Stage> + '_ {
        self.by_stage.keys().copied()
    }

    /// The records of `stage`, in no particular order.
    fn of(&self, stage: Stage) -> impl Iterator<Item = &Deferred> {
        self.by_stage.get(&stage).into_iter().flat_map(|keys| keys.values()).filter_map(|n| {
            looked_at(1);
            self.by_order.get(n)
        })
    }

    /// Is `stage` recorded with cache key `key`?
    fn contains(&self, stage: Stage, key: u64) -> bool {
        looked_at(1);
        self.by_stage.get(&stage).is_some_and(|keys| keys.contains_key(&key))
    }

    /// Adds `d` as the newest record, in place of the record of its stage and key.
    fn push(&mut self, d: Deferred) {
        let order = self.next;
        self.next = self.next.wrapping_add(1);
        if let Some(old) = self.by_stage.entry(d.stage).or_default().insert(d.key, order)
            && let Some(old) = self.by_order.remove(&old)
        {
            unmark(old.key);
        }
        self.by_order.insert(order, d);
    }

    /// Drops the records of `stage` whose key `drop` gives true for.
    fn drop_where(&mut self, stage: Stage, drop: impl Fn(u64) -> bool) {
        let Some(keys) = self.by_stage.get_mut(&stage) else { return };
        looked_at(keys.len());
        let by_order = &mut self.by_order;
        keys.retain(|&key, order| {
            let keep = !drop(key);
            if !keep && let Some(d) = by_order.remove(order) {
                unmark(d.key);
            }
            keep
        });
        if keys.is_empty() {
            self.by_stage.remove(&stage);
        }
    }

    /// Drops the records for which `keep` is false.
    fn retain(&mut self, mut keep: impl FnMut(&Deferred) -> bool) {
        let by_stage = &mut self.by_stage;
        self.by_order.retain(|_, d| {
            looked_at(1);
            let k = keep(d);
            if !k {
                if let Some(keys) = by_stage.get_mut(&d.stage) {
                    keys.remove(&d.key);
                    if keys.is_empty() {
                        by_stage.remove(&d.stage);
                    }
                }
                unmark(d.key);
            }
            k
        });
    }
}

#[cfg(test)]
thread_local! {
    /// How many records the lookups on this thread have looked at.
    static LOOKED_AT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Counts `n` records looked at, in tests.
fn looked_at(n: usize) {
    #[cfg(test)]
    LOOKED_AT.with(|c| c.set(c.get() + n));
    #[cfg(not(test))]
    let _ = n;
}

struct State {
    phase: Phase,
    deferred: Records,
    /// How many stages the first view left deferred, until [`Deferrals::take_report`] takes it.
    report: Option<usize>,
    /// The refresh running, if one is ([`Deferrals::refresh`]).
    refresh: Option<Refresh>,
    clock: Clock,
}

/// A [`Deferrals::refresh`] while it runs. It applies only to the thread that runs it: drawings
/// of the document on other threads go on as if no refresh ran.
struct Refresh {
    thread: std::thread::ThreadId,
    /// When its budget started (`None`: no run yet).
    started: Option<u64>,
    /// The objects whose stages run again; the others' stay deferred.
    ids: HashSet<u64>,
    /// The cache keys whose cached failures have been run again.
    retried: HashSet<u64>,
    /// The stages of `ids` that were drawn.
    visited: HashSet<Stage>,
    /// The stage inputs of `ids` that ran, or were taken from the cache, or were deferred again.
    touched: HashSet<(Stage, u64)>,
}

impl State {
    /// The refresh running on this thread.
    fn refresh_here(&mut self) -> Option<&mut Refresh> {
        let here = std::thread::current().id();
        self.refresh.as_mut().filter(|r| r.thread == here)
    }

    /// The phase that applies on this thread: a refresh running here has a budget of its own.
    fn phase_here(&mut self) -> Phase {
        match self.refresh_here() {
            Some(r) => Phase::Budgeted { started: r.started, first: false },
            None => self.phase,
        }
    }

    /// Is a refresh running here, and does it run the stages of `object`?
    fn refreshes(&mut self, object: u64) -> Option<bool> {
        self.refresh_here().map(|r| r.ids.contains(&object))
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for d in self.deferred.iter() {
            unmark(d.key);
        }
    }
}

/// The deferred stages and the opening budget of one open document. Clones share them.
#[derive(Clone)]
pub struct Deferrals(Arc<Mutex<State>>);

impl Default for Deferrals {
    fn default() -> Self {
        Self::with_clock(Arc::new(monotonic))
    }
}

impl std::fmt::Debug for Deferrals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deferrals").field("deferred", &self.list()).finish()
    }
}

/// What a budgeted run may take.
pub(crate) enum Allowance {
    /// No budget applies: the per-call limit.
    Unlimited,
    /// At most this many milliseconds, which end the budget at `until`.
    Until { ms: u64, until: u64 },
    /// The budget is spent: the stage is deferred without running.
    Spent,
}

impl Deferrals {
    /// A document's deferrals before its first view, timed by the monotonic clock.
    pub fn new() -> Self {
        Self::default()
    }

    /// [`Deferrals::new`] timed by `clock` (tests give a clock of their own).
    pub fn with_clock(clock: Clock) -> Self {
        Self(Arc::new(Mutex::new(State {
            phase: Phase::Budgeted { started: None, first: true },
            deferred: Records::default(),
            report: None,
            refresh: None,
            clock,
        })))
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The deferred stages, by object and effect, each once (a stage drawn with several inputs,
    /// such as the paths of a group, is listed with the first one's reason).
    pub fn list(&self) -> Vec<Deferred> {
        let mut out: Vec<Deferred> = self.state().deferred.iter().cloned().collect();
        out.sort_by_key(|d| (d.stage.object, d.stage.item, d.stage.effect));
        out.dedup_by_key(|d| d.stage);
        out
    }

    /// Is the first view still being drawn ([`Deferrals::first_view_done`] not called yet)?
    pub fn opening(&self) -> bool {
        matches!(self.state().phase, Phase::Budgeted { first: true, .. })
    }

    /// Runs `f` with these deferrals applied to the plug-in effects evaluated on this thread.
    pub fn view<T>(&self, f: impl FnOnce() -> T) -> T {
        let previous = VIEW.with(|v| v.borrow_mut().replace(self.clone()));
        let _ = PASSES.try_with(|p| p.borrow_mut().push(Pass::default()));
        let _restore = Restore { previous, current: self.clone() };
        f()
    }

    /// One drawing inside [`Deferrals::view`] has ended: a stage that ran after the first view,
    /// whether it gave a result or was deferred again, keeps only the records of the inputs that
    /// drawing left deferred. The records of its other inputs were made with input it no longer
    /// has.
    fn settle(&self, pass: Pass) {
        if pass.ran.is_empty() {
            return;
        }
        let mut s = self.state();
        for &stage in &pass.ran {
            s.deferred.drop_where(stage, |key| !pass.seen.contains(&(stage, key)));
        }
    }

    /// The first view has been drawn: later drawings leave deferred stages deferred, and
    /// [`Deferrals::take_report`] gives how many there are. Does nothing after the first call.
    pub fn first_view_done(&self) {
        let mut s = self.state();
        if let Phase::Budgeted { first: true, .. } = s.phase {
            s.phase = Phase::Open;
            s.report = Some(s.stages().len());
        }
    }

    /// How many stages the first view left deferred, once, after it was drawn.
    pub fn take_report(&self) -> Option<usize> {
        self.state().report.take()
    }

    /// Runs the deferred stages of the objects in `objects` (all of them when `None`) again with a
    /// fresh budget: `run` is given those objects, draws them inside [`Deferrals::view`] on this
    /// thread, and returns the objects it drew whole, every part of them reached. A stage the
    /// drawing does not reach (that of a hidden object, say) stays deferred as it was, and so does
    /// a stage input it does not reach, unless the object was drawn whole: the input is then one
    /// the stage no longer has. The refresh and its budget apply only to this thread.
    pub fn refresh(&self, objects: Option<&[u64]>, run: impl FnOnce(&[u64]) -> Vec<u64>) -> Refreshed {
        let (targets, ids) = {
            let mut s = self.state();
            let asked: Option<HashSet<u64>> = objects.map(|o| o.iter().copied().collect());
            let mut targets: Vec<Stage> = s.deferred.stages().filter(|t| asked.as_ref().is_none_or(|o| o.contains(&t.object))).collect();
            targets.sort_by_key(|t| (t.object, t.item, t.effect));
            let mut ids: Vec<u64> = targets.iter().map(|t| t.object).collect();
            ids.dedup();
            let thread = std::thread::current().id();
            let (retried, visited, touched) = (HashSet::new(), HashSet::new(), HashSet::new());
            s.refresh = Some(Refresh { thread, started: None, ids: ids.iter().copied().collect(), retried, visited, touched });
            (targets, ids)
        };
        // The refresh ends when `run` returns, also when it unwinds.
        struct Reopen<'a>(&'a Deferrals);
        impl Drop for Reopen<'_> {
            fn drop(&mut self) {
                self.0.state().refresh = None;
            }
        }
        let reopen = Reopen(self);
        let whole: HashSet<u64> = run(&ids).into_iter().collect();
        let mut s = self.state();
        let touched = match s.refresh.take() {
            Some(r) => {
                for &stage in r.visited.iter().filter(|t| whole.contains(&t.object)) {
                    s.deferred.drop_where(stage, |key| !r.touched.contains(&(stage, key)));
                }
                r.touched
            }
            None => HashSet::new(),
        };
        drop(s);
        drop(reopen);
        // A stage fails again when an input it ran with this time is deferred for a reason other
        // than time; one with records left that this refresh did not run is still deferred.
        let s = self.state();
        let mut out = Refreshed::default();
        for &t in &targets {
            let mut left = s.deferred.of(t).peekable();
            if left.peek().is_none() {
                out.refreshed += 1;
            } else if left.any(|d| d.reason != Reason::Budget && touched.contains(&(t, d.key))) {
                out.failed += 1;
            } else {
                out.still_deferred += 1;
            }
        }
        drop(s);
        if out.refreshed > 0 {
            REVISION.fetch_add(1, Ordering::Relaxed);
        }
        out
    }

    /// Records `stage` with cache key `key` (of plug-in `plugin`) as deferred for `reason`, in
    /// place of what was recorded for that stage and key. After the first view, when the drawing
    /// ends, the stage's records of inputs that drawing did not leave deferred are dropped
    /// ([`Deferrals::settle`]).
    pub(crate) fn defer(&self, stage: Stage, plugin: &str, reason: Reason, key: u64) {
        seen(stage, key);
        let mut s = self.state();
        s.remove(stage, key);
        mark(key);
        s.deferred.push(Deferred { stage, plugin: plugin.to_string(), reason, key });
        s.ran_here(stage, key);
    }

    /// `stage` gave a result for cache key `key`, from a run or from the cache: it is no longer
    /// deferred with that input. After the first view, when the drawing ends, the stage's records
    /// of inputs that drawing did not leave deferred are dropped as well ([`Deferrals::settle`]).
    pub(crate) fn ran(&self, stage: Stage, key: u64) {
        let mut s = self.state();
        s.remove(stage, key);
        s.ran_here(stage, key);
    }

    /// Should `stage`, with cache key `key`, be left deferred without running? After the first
    /// view, a stage deferred with the same input and parameters is, and so is one of an object a
    /// refresh does not run.
    pub(crate) fn holds(&self, stage: Stage, key: u64) -> bool {
        let mut s = self.state();
        let settled = s.phase_here() == Phase::Open || s.refreshes(stage.object) == Some(false);
        let held = settled && s.deferred.contains(stage, key);
        drop(s);
        if held {
            seen(stage, key);
        }
        held
    }

    /// `stage` is being drawn: a refresh running here that runs its object's stages notes that it
    /// reached the stage.
    pub(crate) fn visit(&self, stage: Stage) {
        let mut s = self.state();
        if let Some(r) = s.refresh_here().filter(|r| r.ids.contains(&stage.object)) {
            r.visited.insert(stage);
        }
    }

    /// Drops the records of the stages for which `keep` (given the stage and the plug-in id) is
    /// false, such as those of effects or objects removed from the document.
    pub fn retain(&self, keep: impl Fn(Stage, &str) -> bool) {
        self.state().deferred.retain(|d| keep(d.stage, &d.plugin));
    }

    /// Should the failure cached for `stage` with cache key `key` be run again? Yes once per key
    /// while a refresh running here runs the stages of `stage`'s object.
    pub(crate) fn retries(&self, stage: Stage, key: u64) -> bool {
        let mut s = self.state();
        s.refresh_here().filter(|r| r.ids.contains(&stage.object)).is_some_and(|r| r.retried.insert(key))
    }

    /// What a run starting now may take of the budget: the budget starts with the first run.
    /// Without a clock, the first view runs nothing and a refresh runs every stage under its
    /// per-call limit.
    pub(crate) fn allowance(&self) -> Allowance {
        let mut s = self.state();
        let Phase::Budgeted { started, first } = s.phase_here() else { return Allowance::Unlimited };
        let Some(now) = (s.clock)() else { return if first { Allowance::Spent } else { Allowance::Unlimited } };
        let start = started.unwrap_or(now);
        match s.refresh_here() {
            Some(r) => r.started = Some(start),
            None => s.phase = Phase::Budgeted { started: Some(start), first },
        }
        let until = start.saturating_add(OPENING_BUDGET_MS);
        match until.checked_sub(now).filter(|ms| *ms > 0) {
            Some(ms) => Allowance::Until { ms, until },
            None => Allowance::Spent,
        }
    }

    /// Has the clock passed `until` (a result that came back now is too late)? Without a clock,
    /// yes.
    pub(crate) fn passed(&self, until: u64) -> bool {
        let clock = self.state().clock.clone();
        clock().is_none_or(|now| now > until)
    }
}

impl State {
    /// Removes what is recorded for `stage` with cache key `key`. A record without a key (the
    /// plug-in was not looked at) stands for any input, so it goes whatever `key` is, and `key` 0
    /// removes every record of the stage.
    fn remove(&mut self, stage: Stage, key: u64) {
        self.deferred.drop_where(stage, |k| k == key || k == 0 || key == 0);
    }

    /// `stage` was run with cache key `key`, or taken from the cache, by the drawing on this
    /// thread: a refresh running here notes the input ([`Deferrals::refresh`]), and after the first
    /// view [`Deferrals::settle`] looks at the stage when that drawing ends.
    fn ran_here(&mut self, stage: Stage, key: u64) {
        if let Some(r) = self.refresh_here().filter(|r| r.ids.contains(&stage.object)) {
            r.touched.insert((stage, key));
        }
        if self.phase_here() == Phase::Open {
            let _ = PASSES.try_with(|p| {
                if let Some(pass) = p.borrow_mut().last_mut() {
                    pass.ran.insert(stage);
                }
            });
        }
    }

    /// The deferred stages, each once.
    fn stages(&self) -> Vec<Stage> {
        let mut out: Vec<Stage> = self.deferred.stages().collect();
        out.sort_by_key(|t| (t.object, t.item, t.effect));
        out
    }
}

thread_local! {
    static VIEW: RefCell<Option<Deferrals>> = const { RefCell::new(None) };
    /// One entry for each [`Deferrals::view`] running on this thread, innermost last.
    static PASSES: RefCell<Vec<Pass>> = const { RefCell::new(Vec::new()) };
}

/// What one drawing inside a view did: the stages that ran after the first view (with a result or
/// deferred again), and the stage inputs it left deferred.
#[derive(Default)]
struct Pass {
    ran: HashSet<Stage>,
    seen: HashSet<(Stage, u64)>,
}

/// The drawing on this thread left `stage` deferred with cache key `key`.
fn seen(stage: Stage, key: u64) {
    let _ = PASSES.try_with(|p| {
        if let Some(pass) = p.borrow_mut().last_mut() {
            pass.seen.insert((stage, key));
        }
    });
}

/// Ends the drawing and puts the previous view back, also when the work inside unwinds.
struct Restore {
    previous: Option<Deferrals>,
    current: Deferrals,
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Some(pass) = PASSES.try_with(|p| p.borrow_mut().pop()).ok().flatten() {
            self.current.settle(pass);
        }
        let previous = self.previous.take();
        let _ = VIEW.try_with(|v| *v.borrow_mut() = previous);
    }
}

/// The deferrals of the view being drawn on this thread ([`Deferrals::view`]).
pub fn current() -> Option<Deferrals> {
    VIEW.try_with(|v| v.borrow().clone()).ok().flatten()
}

/// Changes when deferred stages give results: [`Deferrals::refresh`] ran some, or a run outside a
/// view gave a result for a stage some document has deferred.
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

static REVISION: AtomicU64 = AtomicU64::new(0);

/// The cache keys some document's deferred stages were deferred with, and how many stages hold each.
static MARKED: Mutex<Option<HashMap<u64, usize>>> = Mutex::new(None);

fn mark(key: u64) {
    if key != 0 {
        *MARKED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).entry(key).or_default() += 1;
    }
}

fn unmark(key: u64) {
    let mut marked = MARKED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(m) = marked.as_mut() else { return };
    if let Some(n) = m.get_mut(&key) {
        *n -= 1;
        if *n == 0 {
            m.remove(&key);
        }
    }
}

/// A run gave a result for cache key `key`: when some document deferred a stage with that key,
/// [`revision`] changes.
pub(crate) fn resolved(key: u64) {
    let marked = MARKED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|m| m.contains_key(&key));
    if marked {
        REVISION.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock that reads `step` milliseconds later each time it is read.
    fn stepping(step: u64) -> Clock {
        let t = Arc::new(AtomicU64::new(0));
        Arc::new(move || Some(t.fetch_add(step, Ordering::Relaxed)))
    }

    #[test]
    fn the_budget_starts_with_the_first_run_and_ends_after_it() {
        let d = Deferrals::with_clock(stepping(700));
        assert!(matches!(d.allowance(), Allowance::Until { ms: 2_000, until: 2_000 }));
        assert!(matches!(d.allowance(), Allowance::Until { ms: 1_300, until: 2_000 }));
        assert!(matches!(d.allowance(), Allowance::Until { ms: 600, until: 2_000 }));
        assert!(matches!(d.allowance(), Allowance::Spent));
        d.first_view_done();
        assert!(matches!(d.allowance(), Allowance::Unlimited));
    }

    #[test]
    fn without_a_clock_the_first_view_runs_nothing_and_a_refresh_everything() {
        let d = Deferrals::with_clock(Arc::new(|| None));
        assert!(matches!(d.allowance(), Allowance::Spent));
        assert!(d.passed(u64::MAX));
        d.first_view_done();
        d.defer(Stage::default(), "a", Reason::Budget, 31);
        d.refresh(None, |_| {
            assert!(matches!(d.allowance(), Allowance::Unlimited));
            vec![]
        });
    }

    /// The default clock, which the web build also reads, gives the first view its budget.
    #[test]
    fn the_default_clock_gives_the_first_view_a_budget() {
        let d = Deferrals::new();
        assert!(matches!(d.allowance(), Allowance::Until { ms: OPENING_BUDGET_MS, .. }));
        assert!(!d.passed(u64::MAX));
    }

    /// A stage drawn with several inputs (the paths of a group) holds one record for each, and is
    /// listed and counted once.
    #[test]
    fn a_stage_is_listed_and_counted_once() {
        let d = Deferrals::with_clock(stepping(1));
        let s = Stage { object: 7, effect: 1, item: None };
        d.defer(s, "a", Reason::Budget, 11);
        d.defer(s, "a", Reason::Budget, 12);
        d.defer(s, "a", Reason::Timeout, 12);
        d.defer(Stage { item: Some(0), ..s }, "a", Reason::Failed, 13);
        assert_eq!(d.list().len(), 2);
        assert!(!d.holds(s, 12), "the first view runs deferred stages");
        d.first_view_done();
        assert!(d.holds(s, 11) && d.holds(s, 12));
        assert!(!d.holds(s, 14), "other input");
        assert_eq!(d.take_report(), Some(2));
        assert_eq!(d.take_report(), None);
        d.first_view_done();
        assert_eq!(d.take_report(), None, "the first view ends once");
        d.ran(s, 11);
        assert_eq!(d.list().len(), 2, "the stage is still deferred with its other input");
        d.ran(s, 12);
        assert_eq!(d.list().len(), 1);
        d.defer(s, "a", Reason::Missing, 0);
        d.ran(s, 15);
        assert_eq!(d.list().len(), 1, "a record without a key goes with any result");
    }

    /// A refresh runs the deferred stages of the objects asked for with a fresh budget, and counts
    /// what became of them; the stages its drawing did not reach stay as they were. Another
    /// thread draws the document as if no refresh ran.
    #[test]
    fn a_refresh_counts_what_became_of_each_stage() {
        let d = Deferrals::with_clock(stepping(1));
        d.first_view_done();
        let st = |object, effect| Stage { object, effect, item: None };
        let stages = [
            (st(1, 0), Reason::Budget),
            (st(1, 1), Reason::Budget),
            (st(1, 2), Reason::Budget),
            (st(1, 3), Reason::Timeout),
            (st(2, 0), Reason::Budget),
            (st(3, 0), Reason::Failed),
        ];
        for (stage, reason) in stages {
            d.defer(stage, "a", reason, 40 + stage.object * 4 + stage.effect as u64);
        }
        let seen = std::cell::RefCell::new(vec![]);
        let counts = d.refresh(Some(&[1, 3, 9]), |ids| {
            seen.borrow_mut().extend_from_slice(ids);
            assert!(matches!(d.allowance(), Allowance::Until { .. }), "a fresh budget");
            assert!(d.holds(st(2, 0), 48), "an object not asked for stays deferred");
            d.visit(st(2, 0));
            d.visit(st(1, 0));
            d.visit(st(1, 1));
            d.defer(st(1, 1), "a", Reason::Budget, 45);
            std::thread::scope(|scope| {
                let other = scope.spawn(|| (matches!(d.allowance(), Allowance::Unlimited), d.holds(st(1, 2), 46), d.retries(st(3, 0), 52)));
                assert_eq!(other.join().unwrap(), (true, true, false), "another thread");
            });
            d.visit(st(1, 1));
            assert_eq!(d.list().iter().filter(|x| x.stage == st(1, 1)).count(), 1, "a stage is reached once");
            d.visit(st(3, 0));
            d.defer(st(3, 0), "a", Reason::Failed, 52);
            ids.to_vec()
        });
        assert_eq!(*seen.borrow(), [1, 3]);
        // st(1, 3) timed out before and was not reached: still deferred, not failed again.
        assert_eq!(counts, Refreshed { refreshed: 1, still_deferred: 3, failed: 1 });
        assert!(matches!(d.allowance(), Allowance::Unlimited), "the budget ends with the refresh");
        let left = d.list().iter().map(|x| x.stage).collect::<Vec<_>>();
        assert_eq!(left, [st(1, 1), st(1, 2), st(1, 3), st(2, 0), st(3, 0)]);

        // An object the drawing did not reach whole keeps the inputs it did not reach: a reached
        // stage of it gives a result only for the inputs that ran.
        d.defer(st(1, 1), "a", Reason::Budget, 70);
        let counts = d.refresh(Some(&[1]), |_| {
            d.visit(st(1, 1));
            d.ran(st(1, 1), 45);
            vec![]
        });
        assert_eq!(counts, Refreshed { refreshed: 0, still_deferred: 3, failed: 0 });
        let keys = |stage| d.state().deferred.iter().filter(|x| x.stage == stage).map(|x| x.key).collect::<Vec<_>>();
        assert_eq!(keys(st(1, 1)), [70]);
        let counts = d.refresh(Some(&[1]), |_| {
            d.visit(st(1, 1));
            vec![1]
        });
        assert_eq!(counts, Refreshed { refreshed: 1, still_deferred: 2, failed: 0 }, "drawn whole: an input it no longer has");
        assert_eq!(keys(st(1, 1)), Vec::<u64>::new());
        d.retain(|stage, _| stage.object != 1);
        assert_eq!(d.list().iter().map(|x| x.stage).collect::<Vec<_>>(), [st(2, 0), st(3, 0)]);
    }

    /// Objects asked for or drawn whole more than once count once; a stage with several inputs
    /// gives a result when its object is drawn whole, fails when an input it ran with fails again
    /// (even beside an older failure it did not run), and stays deferred when it is reached but
    /// not drawn whole or not reached at all.
    #[test]
    fn a_refresh_with_repeated_objects_and_several_inputs_per_stage() {
        const K: u64 = 0xdef1_0000;
        let marked = |key| MARKED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|m| m.contains_key(&key));
        let d = Deferrals::with_clock(stepping(1));
        d.first_view_done();
        let st = |object| Stage { object, effect: 0, item: None };
        for (object, reason, key) in [
            (10, Reason::Budget, K + 1),
            (10, Reason::Budget, K + 2),
            (11, Reason::Timeout, K + 3),
            (11, Reason::Budget, K + 4),
            (12, Reason::Timeout, K + 5),
            (13, Reason::Budget, K + 6),
            (14, Reason::Budget, K + 7),
        ] {
            d.defer(st(object), "a", reason, key);
        }
        let before = revision();
        let seen = std::cell::RefCell::new(vec![]);
        let counts = d.refresh(Some(&[10, 11, 12, 13, 10, 11, 99]), |ids| {
            seen.borrow_mut().extend_from_slice(ids);
            d.visit(st(10));
            d.visit(st(10));
            d.ran(st(10), K + 1);
            d.visit(st(11));
            d.defer(st(11), "a", Reason::Failed, K + 8);
            d.visit(st(12));
            vec![10, 13, 10, 13]
        });
        assert_eq!(*seen.borrow(), [10, 11, 12, 13]);
        assert_eq!(counts, Refreshed { refreshed: 1, still_deferred: 2, failed: 1 });
        assert!(revision() > before);
        let keys = || d.state().deferred.iter().map(|x| (x.stage.object, x.key)).collect::<Vec<_>>();
        assert_eq!(keys(), [(11, K + 3), (11, K + 4), (12, K + 5), (13, K + 6), (14, K + 7), (11, K + 8)]);
        assert!(!marked(K + 1) && !marked(K + 2), "the records of the stage that gave a result");
        assert!(marked(K + 3) && marked(K + 8));

        // Again, with nothing reached: every stage asked for stays as it was.
        let counts = d.refresh(Some(&[11, 12, 12]), |_| vec![11, 12]);
        assert_eq!(counts, Refreshed { refreshed: 0, still_deferred: 2, failed: 0 });
        assert_eq!(keys(), [(11, K + 3), (11, K + 4), (12, K + 5), (13, K + 6), (14, K + 7), (11, K + 8)]);
    }

    /// A refresh whose drawing unwinds ends: later drawings on this thread are no longer part of
    /// it, and the next refresh has a budget of its own.
    #[test]
    fn a_refresh_that_unwinds_ends() {
        let d = Deferrals::with_clock(stepping(1));
        d.first_view_done();
        let s = Stage { object: 5, effect: 0, item: None };
        d.defer(s, "a", Reason::Budget, 0xdef2_0001);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            d.refresh(None, |_| {
                assert!(!d.holds(s, 0xdef2_0001));
                panic!("the drawing stopped")
            })
        }));
        assert!(unwound.is_err());
        assert!(matches!(d.allowance(), Allowance::Unlimited));
        assert!(d.holds(s, 0xdef2_0001), "the stage stays deferred");
        let counts = d.refresh(Some(&[5]), |_| {
            assert!(matches!(d.allowance(), Allowance::Until { .. }));
            d.visit(s);
            vec![5]
        });
        assert_eq!(counts, Refreshed { refreshed: 1, still_deferred: 0, failed: 0 });
    }

    /// After the first view, a stage drawn with several inputs (the paths of a group) whose one
    /// changed input runs keeps the records of the inputs the same drawing left deferred, and drops
    /// the record of the input it no longer has.
    #[test]
    fn a_stage_that_runs_keeps_the_inputs_its_drawing_left_deferred() {
        let d = Deferrals::with_clock(stepping(1));
        let s = Stage { object: 4, effect: 0, item: None };
        let other = Stage { object: 5, ..s };
        for key in [61, 62] {
            d.defer(s, "a", Reason::Budget, key);
        }
        d.defer(other, "a", Reason::Budget, 63);
        d.first_view_done();
        d.view(|| {
            assert!(d.holds(s, 61), "an unchanged path");
            assert!(!d.holds(s, 64), "the changed path");
            d.ran(s, 64);
        });
        let keys = d.state().deferred.iter().map(|x| (x.stage.object, x.key)).collect::<Vec<_>>();
        assert_eq!(keys, [(4, 61), (5, 63)]);

        // A changed input that fails again replaces the record of the input it no longer has.
        d.view(|| {
            assert!(!d.holds(other, 65));
            d.defer(other, "a", Reason::Failed, 65);
        });
        let left = d.state().deferred.iter().map(|x| (x.stage.object, x.key, x.reason)).collect::<Vec<_>>();
        assert_eq!(left, [(4, 61, Reason::Budget), (5, 65, Reason::Failed)]);
    }

    #[test]
    fn a_result_for_a_deferred_key_changes_the_revision() {
        const KEY: u64 = 0xdef0_0001;
        let marked = || MARKED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|m| m.contains_key(&KEY));
        let d = Deferrals::with_clock(stepping(1));
        assert!(!marked());
        d.defer(Stage::default(), "a", Reason::Budget, KEY);
        let other = d.clone();
        other.defer(Stage { object: 2, ..Stage::default() }, "a", Reason::Budget, KEY);
        assert!(marked());
        let before = revision();
        resolved(KEY);
        assert!(revision() > before);
        other.ran(Stage::default(), KEY);
        assert!(marked(), "another stage holds the key");
        drop((d, other));
        assert!(!marked(), "the document closed");
    }

    /// Deferrals after the first view with `n` stages, one input each, keyed from `base`.
    fn many(n: u64, base: u64) -> Deferrals {
        let d = Deferrals::with_clock(stepping(1));
        for object in 0..n {
            d.defer(Stage { object, effect: 0, item: None }, "a", Reason::Budget, base + object);
        }
        d.first_view_done();
        d
    }

    /// How many records `f` looks at on this thread.
    fn looked_at_by(f: impl FnOnce()) -> usize {
        let before = LOOKED_AT.with(std::cell::Cell::get);
        f();
        LOOKED_AT.with(std::cell::Cell::get) - before
    }

    /// Asking whether a stage input stays deferred looks at that stage's records only, however
    /// many other stages are deferred.
    #[test]
    fn holding_a_stage_looks_at_its_own_records_only() {
        const K: u64 = 0xdef3_0000;
        let d = many(2_000, K);
        let s = Stage { object: 1_000, effect: 0, item: None };
        let n = looked_at_by(|| {
            assert!(d.holds(s, K + 1_000));
            assert!(!d.holds(s, K + 1_001), "another stage's input");
            assert!(!d.holds(Stage { effect: 1, ..s }, K + 1_000), "another stage");
        });
        assert!(n <= 3, "looked at {n} records");
    }

    /// A stage that gives a result, or is deferred again, replaces its own records only, and so
    /// does the end of a drawing after the first view: the other stages' records are not looked at.
    #[test]
    fn replacing_a_stage_record_looks_at_its_own_records_only() {
        const K: u64 = 0xdef4_0000;
        let d = many(2_000, K);
        let st = |object| Stage { object, effect: 0, item: None };
        let n = looked_at_by(|| {
            d.ran(st(1_000), K + 1_000);
            d.defer(st(1_001), "a", Reason::Failed, K + 9_999);
            d.defer(st(1_002), "a", Reason::Missing, 0);
            d.ran(st(1_002), 7);
        });
        assert!(n <= 6, "looked at {n} records");
        let n = looked_at_by(|| {
            d.view(|| {
                assert!(!d.holds(st(1_003), K + 9_998));
                d.ran(st(1_003), K + 9_998);
            });
        });
        assert!(n <= 4, "looked at {n} records");
        let keys = d.state().deferred.iter().filter(|x| (1_000..1_004).contains(&x.stage.object)).map(|x| x.key).collect::<Vec<_>>();
        assert_eq!(keys, [K + 1_001, K + 9_999]);
        assert_eq!(d.list().len(), 1_997);
    }
}
